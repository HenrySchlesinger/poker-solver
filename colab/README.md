# Colab notebooks

Jupyter notebooks that run offline precompute. See
[../docs/COLAB.md](../docs/COLAB.md) for the full strategy.

Files in this directory are **Markdown plans**, not actual `.ipynb`
files. Convert to notebooks via `jupytext` or paste cells into Colab
directly. Keeping plans in Markdown means they live nicely in git.

## Notebooks

- [precompute_preflop.md](precompute_preflop.md) — one-time job to
  generate the shipped preflop range database
- [precompute_flops.md](precompute_flops.md) — the big overnight job
  that populates the flop cache
- [convergence_bench.md](convergence_bench.md) — diff our outputs against
  TexasSolver on canonical spots (nightly CI-like)

## Running

Open a new Colab notebook. Paste the cells from the `.md` file in order.
Save the resulting `.ipynb` locally if you want to re-run later, but
**don't commit `.ipynb` files** — they're noisy in diffs and the
Markdown plan is the source of truth.
