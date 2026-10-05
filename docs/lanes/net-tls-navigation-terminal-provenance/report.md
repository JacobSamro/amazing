Preserve saved navigation-owner authority and exact terminal CSP provenance. Manual continuations retain original settings; original terminal failures and reporting decisions preserve the actual receiving owner.

Integrator tested exact `31893bde` on fixed base `fd87ab4c`; shipping engine/Cargo is byte-exact on tooling base `d569d7f5`. Workspace: 4,885 passing, 0 failing; doctests: 30 passing. Native network corpus: 598 → 615, zero losses. Original 11 CSP scopes: 32 files / 81 assertions, zero losses. Full 16-shard Test262: 52,701 passing files / 101,215 passing variants, zero lost files or variants against exact `408cb841`. Four native pools completed with actual exit 0 and authenticated compact fetch 0; jobs remain 4 and timeout remains 60 seconds per original shard.

Original failed attempts and the published baseline’s collector incident remain recorded. Source-loan and Reporting 410 follow-up features are separate, unqualified work.
