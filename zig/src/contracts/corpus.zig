//! Zig verifier for the vendored Abbey Program 1 corpus, transcribing
//! `scripts/check-abbey-contracts.py` (copied unchanged from the oracle):
//! pinned lock and manifest identity, exact inventory (no symlinks, no extra
//! or missing files), per-artifact length and SHA-256, the privacy taxonomy
//! scan, and the domain-separated aggregate digest over the manifest rows plus
//! the manifest itself re-rendered with a zeroed aggregate.
const std = @import("std");
const pretty = @import("../json/pretty.zig");
const Allocator = std.mem.Allocator;
const Sha256 = std.crypto.hash.sha2.Sha256;

pub const pinned_repository = "https://github.com/donaldfilimon/abi";
pub const pinned_revision = "348754bdaaf59a40fbb858380f925e0aba95a23b";
pub const pinned_digest = "72e241e34967df318376bf68f4a0e2db13f5ebf17d1a219709731f1f470dbe8e";
pub const pinned_artifact_count = 81;
pub const pinned_total_bytes = 88_328;
const max_artifact_bytes = 1024 * 1024;
const max_corpus_bytes = 16 * 1024 * 1024;
const aggregate_domain = "abbey-contract-corpus-v1\x00";
const forbidden_keys = [_][]const u8{ "audio", "transcript", "message", "prompt", "response_text", "credential", "token", "password", "username", "display_name", "filesystem_path", "participant_identity" };

/// Closed reason codes, the Python guard's vocabulary.
pub const Failure = enum {
    lock_shape,
    lock_pin_mismatch,
    corpus_missing,
    manifest_shape,
    manifest_identity_mismatch,
    manifest_artifact_shape,
    manifest_duplicate_artifact,
    symlink_forbidden,
    artifact_not_regular,
    artifact_unreadable,
    corpus_inventory_mismatch,
    fixture_shape,
    privacy_taxonomy_mismatch,
    artifact_length_mismatch,
    artifact_digest_mismatch,
    corpus_too_large,
    aggregate_digest_mismatch,
    corpus_identity_mismatch,
    json_invalid,
};

pub const Result = union(enum) {
    ok: struct { artifacts: usize, total_bytes: usize },
    failed: struct { reason: Failure, path: []const u8 },
};

fn fail(reason: Failure, path: []const u8) Result {
    return .{ .failed = .{ .reason = reason, .path = path } };
}

fn hex(digest: [32]u8) [64]u8 {
    return std.fmt.bytesToHex(digest, .lower);
}

fn parseStrict(arena: Allocator, bytes: []const u8) ?std.json.Value {
    // Python's guard rejects duplicate members; std.json's default does too.
    return std.json.parseFromSliceLeaky(std.json.Value, arena, bytes, .{ .duplicate_field_behavior = .@"error" }) catch null;
}

fn keysExactly(obj: std.json.ObjectMap, want: []const []const u8) bool {
    if (obj.count() != want.len) return false;
    for (want) |k| if (!obj.contains(k)) return false;
    return true;
}

fn isHex64(s: []const u8) bool {
    if (s.len != 64) return false;
    for (s) |c| if (!(std.ascii.isDigit(c) or (c >= 'a' and c <= 'f'))) return false;
    return true;
}

fn strField(obj: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const v = obj.get(name) orelse return null;
    return if (v == .string) v.string else null;
}

fn intField(obj: std.json.ObjectMap, name: []const u8) ?i64 {
    const v = obj.get(name) orelse return null;
    return if (v == .integer) v.integer else null;
}

const Row = struct { path: []const u8, bytes: u64, sha256: []const u8 };

pub fn verify(arena: Allocator, io: std.Io, root: std.Io.Dir) !Result {
    const lock_bytes = root.readFileAlloc(io, "abbey-contracts.lock.json", arena, .limited(max_artifact_bytes)) catch return fail(.artifact_unreadable, "abbey-contracts.lock.json");
    const lock = parseStrict(arena, lock_bytes) orelse return fail(.json_invalid, "abbey-contracts.lock.json");
    if (lock != .object or !keysExactly(lock.object, &.{ "source_repository", "source_revision", "contract_major", "contract_revision", "aggregate_digest" })) return fail(.lock_shape, "abbey-contracts.lock.json");
    const lo = lock.object;
    if (!eql(strField(lo, "source_repository"), pinned_repository) or !eql(strField(lo, "source_revision"), pinned_revision) or
        intField(lo, "contract_major") != 1 or intField(lo, "contract_revision") != 1 or !eql(strField(lo, "aggregate_digest"), pinned_digest))
        return fail(.lock_pin_mismatch, "abbey-contracts.lock.json");

    var corpus = root.openDir(io, "corpus", .{ .iterate = true, .follow_symlinks = false }) catch return fail(.corpus_missing, "corpus");
    defer corpus.close(io);
    const manifest_bytes = corpus.readFileAlloc(io, "manifest.json", arena, .limited(max_artifact_bytes)) catch return fail(.artifact_unreadable, "corpus/manifest.json");
    const manifest = parseStrict(arena, manifest_bytes) orelse return fail(.json_invalid, "corpus/manifest.json");
    if (manifest != .object or !keysExactly(manifest.object, &.{ "contract_major", "contract_revision", "algorithm", "redaction_profile", "artifacts", "aggregate_digest" })) return fail(.manifest_shape, "corpus/manifest.json");
    const mo = manifest.object;
    if (intField(mo, "contract_major") != 1 or intField(mo, "contract_revision") != 1 or
        !eql(strField(mo, "algorithm"), "abbey-contract-corpus-sha256-v1") or !eql(strField(mo, "redaction_profile"), "abbey-contract-redaction-v1") or
        !eql(strField(mo, "aggregate_digest"), pinned_digest))
        return fail(.manifest_identity_mismatch, "corpus/manifest.json");
    const artifacts = mo.get("artifacts").?;
    if (artifacts != .array) return fail(.manifest_shape, "corpus/manifest.json");

    var rows: std.ArrayList(Row) = .empty;
    var listed = std.StringHashMap(void).init(arena);
    for (artifacts.array.items) |item| {
        if (item != .object) return fail(.manifest_artifact_shape, "corpus/manifest.json");
        const ro = item.object;
        for (ro.keys()) |k| {
            if (!(eql(k, "path") or eql(k, "bytes") or eql(k, "media_type") or eql(k, "sha256") or eql(k, "schema_id"))) return fail(.manifest_artifact_shape, "corpus/manifest.json");
        }
        const path = strField(ro, "path") orelse return fail(.manifest_artifact_shape, "corpus/manifest.json");
        const bytes = intField(ro, "bytes") orelse return fail(.manifest_artifact_shape, path);
        const digest = strField(ro, "sha256") orelse return fail(.manifest_artifact_shape, path);
        if (ro.get("media_type") == null) return fail(.manifest_artifact_shape, path);
        if (bytes < 0 or bytes > max_artifact_bytes or !isHex64(digest)) return fail(.manifest_artifact_shape, path);
        if (!validRelative(path)) return fail(.manifest_artifact_shape, "corpus/manifest.json");
        const gop = try listed.getOrPut(path);
        if (gop.found_existing) return fail(.manifest_duplicate_artifact, path);
        try rows.append(arena, .{ .path = path, .bytes = @intCast(bytes), .sha256 = digest });
    }

    // Inventory: every regular file except manifest.json and .DS_Store.
    var walker = try corpus.walk(arena);
    defer walker.deinit();
    var discovered: usize = 0;
    while (try walker.next(io)) |entry| {
        switch (entry.kind) {
            .directory => continue,
            .sym_link => return fail(.symlink_forbidden, try arena.dupe(u8, entry.path)),
            .file => {},
            else => return fail(.artifact_not_regular, try arena.dupe(u8, entry.path)),
        }
        if (eql(entry.basename, ".DS_Store")) continue;
        if (eql(entry.path, "manifest.json")) continue;
        if (!listed.contains(entry.path)) return fail(.corpus_inventory_mismatch, try arena.dupe(u8, entry.path));
        discovered += 1;
    }
    if (discovered != rows.items.len) return fail(.corpus_inventory_mismatch, "corpus/manifest.json");

    var total: usize = 0;
    for (rows.items) |row| {
        const raw = corpus.readFileAlloc(io, row.path, arena, .limited(max_artifact_bytes + 1)) catch return fail(.artifact_unreadable, row.path);
        total += raw.len;
        if (raw.len != row.bytes) return fail(.artifact_length_mismatch, row.path);
        var d: [32]u8 = undefined;
        Sha256.hash(raw, &d, .{});
        if (!eql(&hex(d), row.sha256)) return fail(.artifact_digest_mismatch, row.path);
        if (std.mem.startsWith(u8, row.path, "v1/fixtures/") and std.mem.endsWith(u8, row.path, ".json")) {
            if (try privacyFailure(arena, row.path, raw)) |f| return fail(f, row.path);
        }
    }
    if (total > max_corpus_bytes) return fail(.corpus_too_large, "corpus");
    const aggregate = try aggregateDigest(arena, rows.items, manifest);
    if (!eql(&aggregate, pinned_digest)) return fail(.aggregate_digest_mismatch, "corpus/manifest.json");
    if (rows.items.len != pinned_artifact_count or total != pinned_total_bytes) return fail(.corpus_identity_mismatch, "corpus/manifest.json");
    return .{ .ok = .{ .artifacts = rows.items.len, .total_bytes = total } };
}

fn eql(a: ?[]const u8, b: []const u8) bool {
    return if (a) |x| std.mem.eql(u8, x, b) else false;
}

fn validRelative(p: []const u8) bool {
    if (p.len == 0 or p[0] == '/' or std.mem.indexOfScalar(u8, p, '\\') != null) return false;
    var it = std.mem.splitScalar(u8, p, '/');
    while (it.next()) |part| {
        if (part.len == 0 or eql(part, ".") or eql(part, "..")) return false;
    }
    return true;
}

/// `_aggregate_digest`: rows plus the manifest re-rendered with a zeroed
/// aggregate (Python `json.dumps(indent=2, ensure_ascii=False) + "\n"`),
/// sorted by UTF-8 path, each hashed as `path\0bytes\0sha256\n`.
fn aggregateDigest(arena: Allocator, rows: []const Row, manifest: std.json.Value) ![64]u8 {
    var zeroed = manifest;
    var obj = try manifest.object.clone(arena);
    const zero_digest: [64]u8 = @splat('0');
    try obj.put(arena, "aggregate_digest", .{ .string = &zero_digest });
    zeroed.object = obj;
    var out: std.Io.Writer.Allocating = .init(arena);
    var j: pretty.Pretty = .{ .w = &out.writer };
    try pretty.writeValue(&j, zeroed);
    try out.writer.writeByte('\n');
    const manifest_render = out.written();
    var md: [32]u8 = undefined;
    Sha256.hash(manifest_render, &md, .{});
    const manifest_hex = hex(md);

    const all = try arena.alloc(Row, rows.len + 1);
    @memcpy(all[0..rows.len], rows);
    all[rows.len] = .{ .path = "manifest.json", .bytes = manifest_render.len, .sha256 = &manifest_hex };
    std.mem.sort(Row, all, {}, struct {
        fn lt(_: void, a: Row, b: Row) bool {
            return std.mem.lessThan(u8, a.path, b.path);
        }
    }.lt);
    var h = Sha256.init(.{});
    h.update(aggregate_domain);
    for (all) |row| {
        h.update(row.path);
        h.update("\x00");
        var nbuf: [24]u8 = undefined;
        h.update(std.fmt.bufPrint(&nbuf, "{d}", .{row.bytes}) catch unreachable);
        h.update("\x00");
        h.update(row.sha256);
        h.update("\n");
    }
    var digest: [32]u8 = undefined;
    h.final(&digest);
    return hex(digest);
}

/// `_verify_privacy_taxonomy` for one fixture.
fn privacyFailure(arena: Allocator, path: []const u8, raw: []const u8) !?Failure {
    // Duplicate members are the Rust verifier's judgment in the oracle; the
    // taxonomy read tolerates them (Python json.loads keeps the last).
    const v = std.json.parseFromSliceLeaky(std.json.Value, arena, raw, .{ .duplicate_field_behavior = .use_last }) catch return .fixture_shape;
    if (v != .object or !keysExactly(v.object, &.{ "case_id", "schema", "expect", "document" })) return .fixture_shape;
    var parts = std.mem.splitScalar(u8, path, '/');
    _ = parts.next();
    _ = parts.next();
    const taxonomy = parts.next() orelse return .fixture_shape;
    const expect = v.object.get("expect").?;
    if (eql(taxonomy, "privacy")) {
        const ok = expect == .string and (eql(expect.string, "forbidden_content") or eql(expect.string, "learning_authority_forbidden") or eql(expect.string, "schema_invalid"));
        return if (ok) null else .privacy_taxonomy_mismatch;
    }
    return if (containsPrivateSentinel(v.object.get("document").?)) .privacy_taxonomy_mismatch else null;
}

fn containsPrivateSentinel(v: std.json.Value) bool {
    switch (v) {
        .object => |o| {
            var it = o.iterator();
            while (it.next()) |e| {
                for (forbidden_keys) |k| if (std.ascii.eqlIgnoreCase(e.key_ptr.*, k)) return true;
                if (containsPrivateSentinel(e.value_ptr.*)) return true;
            }
        },
        .array => |a| for (a.items) |item| if (containsPrivateSentinel(item)) return true,
        .string => |s| {
            const decimal = s.len >= 17 and s.len <= 20 and for (s) |c| {
                if (!std.ascii.isDigit(c)) break false;
            } else true;
            if (decimal) return true;
            for ([_][]const u8{ "/Users/", "/home/", "C:\\", "sk-", "ghp_" }) |p| if (std.mem.startsWith(u8, s, p)) return true;
        },
        else => {},
    }
    return false;
}

const testing = std.testing;

test "corpus verifier: the vendored Abbey corpus matches its pinned lock, inventory, digests and aggregate" {
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    var root = try std.Io.Dir.cwd().openDir(testing.io, "contracts/abbey", .{});
    defer root.close(testing.io);
    const result = try verify(arena.allocator(), testing.io, root);
    switch (result) {
        .ok => |ok| {
            try testing.expectEqual(@as(usize, pinned_artifact_count), ok.artifacts);
            try testing.expectEqual(@as(usize, pinned_total_bytes), ok.total_bytes);
        },
        .failed => |f| {
            std.debug.print("corpus failed: {s} at {s}\n", .{ @tagName(f.reason), f.path });
            return error.TestUnexpectedResult;
        },
    }
}

test "corpus verifier: a modified, extra or missing artifact fails closed" {
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena: std.heap.ArenaAllocator = .init(testing.allocator);
    defer arena.deinit();
    const a = arena.allocator();
    // Copy the corpus into a scratch tree.
    var src = try std.Io.Dir.cwd().openDir(testing.io, "contracts/abbey", .{ .iterate = true });
    defer src.close(testing.io);
    var walker = try src.walk(a);
    defer walker.deinit();
    while (try walker.next(testing.io)) |entry| {
        if (entry.kind == .directory) {
            try tmp.dir.createDirPath(testing.io, entry.path);
        } else {
            const bytes = try src.readFileAlloc(testing.io, entry.path, a, .limited(1 << 20));
            try tmp.dir.writeFile(testing.io, .{ .sub_path = entry.path, .data = bytes });
        }
    }
    try testing.expect((try verify(a, testing.io, tmp.dir)) == .ok);
    // Extra file.
    try tmp.dir.writeFile(testing.io, .{ .sub_path = "corpus/v1/extra.json", .data = "{}" });
    const extra = try verify(a, testing.io, tmp.dir);
    try testing.expectEqual(Failure.corpus_inventory_mismatch, extra.failed.reason);
    try tmp.dir.deleteFile(testing.io, "corpus/v1/extra.json");
    // Modified artifact (same length, one byte different).
    const target = "corpus/v1/fixtures/valid/jcs-vector.json";
    var bytes = try tmp.dir.readFileAlloc(testing.io, target, a, .limited(1 << 20));
    bytes[bytes.len - 2] ^= 0x01;
    try tmp.dir.writeFile(testing.io, .{ .sub_path = target, .data = bytes });
    const modified = try verify(a, testing.io, tmp.dir);
    try testing.expect(modified == .failed);
    // Missing artifact.
    try tmp.dir.deleteFile(testing.io, target);
    const missing = try verify(a, testing.io, tmp.dir);
    try testing.expectEqual(Failure.corpus_inventory_mismatch, missing.failed.reason);
}
