## What this PR does

<!-- One or two sentences. What changed, why. -->

## Benchmark impact

<!--
Paste numbers from `cargo bench -p solver-core` for any perf-sensitive
change. Use the criterion `--baseline` pattern so we see a regression
table, not raw numbers:

    cargo bench -p solver-core -- --save-baseline before
    # ...make your change...
    cargo bench -p solver-core -- --baseline before

If this PR is docs-only / CI-only / a pure refactor with no hot-path
code touched, say so:

    N/A — docs-only change.

If you're regressing a bench by >5%, write a justification here.
-->

## Related roadmap task

<!--
Which day + agent task from docs/ROADMAP.md does this fulfill? Example:

    Day 3, agent A1: `std::simd` f32x8 regret update on river.
-->

## Checklist

- [ ] `cargo fmt --all` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace --release` passes
- [ ] Bench numbers pasted above (or N/A note)
- [ ] No new web frameworks, HTTP servers, async runtimes, or cloud deps
