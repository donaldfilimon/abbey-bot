const std = @import("std");

/// Golden contract files the tests embed. Each is an anonymous import so
/// `@embedFile("golden_<name>")` works from any module without a relative path
/// that escapes `src/`.
const goldens = [_][2][]const u8{
    .{ "golden_routing", "contracts/golden/routing.json" },
    .{ "golden_prompts", "contracts/golden/prompts.json" },
    .{ "golden_tidy", "contracts/golden/tidy.json" },
    .{ "golden_catalog", "contracts/golden/catalog.json" },
    .{ "golden_memory", "contracts/golden/memory.json" },
    .{ "golden_payload", "contracts/catalog/command-payload.json" },
    .{ "golden_grounding", "contracts/golden/grounding.json" },
    .{ "golden_availability", "contracts/golden/availability.json" },
    .{ "golden_episode", "contracts/golden/episode.json" },
};

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const lib_mod = b.addModule("abbey", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    for (goldens) |g| lib_mod.addAnonymousImport(g[0], .{ .root_source_file = b.path(g[1]) });

    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &.{.{ .name = "abbey", .module = lib_mod }},
    });
    const exe = b.addExecutable(.{ .name = "abbey-bot-zig", .root_module = exe_mod });
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    run_cmd.addPassthruArgs();
    const run_step = b.step("run", "Run abbey-bot-zig");
    run_step.dependOn(&run_cmd.step);

    const lib_tests = b.addTest(.{ .root_module = lib_mod });
    const exe_tests = b.addTest(.{ .root_module = exe_mod });
    const test_step = b.step("test", "Run unit tests (leak-checked by the test runner)");
    // std/Build/Step/Run.zig: has_side_effects forces a re-run, so a cached
    // build can never report a stale "tests passed" to the gate.
    const lib_run = b.addRunArtifact(lib_tests);
    lib_run.has_side_effects = true;
    const exe_run = b.addRunArtifact(exe_tests);
    exe_run.has_side_effects = true;
    test_step.dependOn(&lib_run.step);
    test_step.dependOn(&exe_run.step);
}
