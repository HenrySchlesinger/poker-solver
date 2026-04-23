---
name: Bug report
about: Something broke or produced wrong output
title: "[bug] "
labels: bug
assignees: ''
---

## Summary

<!--
One sentence. "solver_solve returns -2 on any all-in river spot"
is useful; "it's broken" is not.
-->

## Environment

- Solver version (`solver_version()` output, or the tag you built):
- macOS version:
- Mac hardware (M1 Pro / M2 Max / Intel / etc.):
- How you consumed the solver (SPM binary target / Xcode bridging
  header / standalone `swiftc` / Rust test / `solver-cli`):
- Poker Panel version, if relevant:

## What happened

<!--
What you saw. Include the return code from solver_solve /
solver_lookup_cached, and the full SolveResult contents if any.
-->

## What you expected

<!--
What should have happened instead. If this is a strategy-accuracy
bug (not a crash), include the TexasSolver reference output too.
-->

## Reproducer

<!--
The minimum HandState that reproduces the bug. Feel free to
anonymize the ranges if they're proprietary, but the pot /
effective_stack / to_act / board / bet_tree_version fields need to
be real.

Example:

    board:             AhKh2s (len=3)
    hero_range:        AA, KK, QQ, AKs  (weights attached)
    villain_range:     22+, A2s+, K9s+
    pot:               100
    effective_stack:   1000
    to_act:            hero (0)
    bet_tree_version:  0
-->

## Logs / stack trace

<!--
If `solver_solve` returned -2 (InternalError), we caught a panic
on the Rust side. Paste the console output if any. A Rust
backtrace from RUST_BACKTRACE=1 is ideal.
-->

## Anything else

<!--
Workarounds you tried. Adjacent spots that DO work. Anything
weird.
-->
