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

The `.ipynb` files are generated from the `.md` plans by our `solver-cli`
tool and committed so Colab's "Open in GitHub" flow works. The Markdown
plans remain the source of truth — edit those, then regenerate:

```bash
cargo run --release -p solver-cli -- md-to-ipynb \
    --input colab/precompute_preflop.md \
    --output colab/precompute_preflop.ipynb
```

## Open in Colab

| Notebook | Open in Colab |
|---|---|
| Preflop precompute | [![Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/HenrySchlesinger/poker-solver/blob/main/colab/precompute_preflop.ipynb) |
| Flop cache grid | [![Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/HenrySchlesinger/poker-solver/blob/main/colab/precompute_flops.ipynb) |
| Convergence validation | [![Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/HenrySchlesinger/poker-solver/blob/main/colab/convergence_bench.ipynb) |
