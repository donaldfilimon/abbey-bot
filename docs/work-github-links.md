# Work GitHub links

Work tasks can hold a typed GitHub issue or pull request reference: installation
ID, repository owner and name, item kind, and number. The repository must be on
the project's manager-approved allowlist. Team members must also retain project
membership and current access to the project's Discord channel to link or read
it. Personal projects remain visible only to their owner.

The canonical work store holds the allowlist, links, and last successful read
snapshots. Older work files load with empty GitHub fields. A manager can explicitly
run `migrate_github_sources` after approving a repository; it recognizes only
canonical issue and pull request URLs with an unambiguous allowed installation.
Other legacy source URLs remain unchanged.

Briefings show a canonical GitHub URL, a bounded title, state, and refresh age.
Untrusted GitHub titles are rendered as inert display text. Failed reads should
call `mark_github_stale`, retaining the last snapshot. A conditional 304 may call
`confirm_github_not_modified` to advance the refresh time. Older observations
cannot replace newer snapshots. These pure store operations do not perform
network I/O or grant authority from repository content.

GitHub App credentials, selected installations, read polling, pagination,
rate-limit handling, and approved issue writes belong to the later GitHub App
operations task. No live GitHub read or write is implied by these source tests.
