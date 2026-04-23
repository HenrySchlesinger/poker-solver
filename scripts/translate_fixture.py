#!/usr/bin/env python3
"""translate_fixture.py — convert our fixture JSON into a TexasSolver config.

Overview
========

Our solver consumes JSON fixtures documented in
``crates/solver-cli/tests/fixtures/SCHEMA.md`` (agent A15 owns the fixtures
and the schema file). Each fixture is one canonical poker spot.

TexasSolver's console binary consumes a line-based imperative config (the
format is documented in its console-branch README — see
https://github.com/bupticybee/TexasSolver/tree/console#usage). Each line is a
single command. A typical file looks roughly like::

    set_pot 50
    set_effective_stack 200
    set_board Qs,Jh,2h
    set_range_ip AA,KK,...
    set_range_oop QQ:0.5,JJ,...
    set_bet_sizes oop,flop,bet,50
    ...
    set_allin_threshold 0.67
    build_tree
    set_thread_num 8
    set_accuracy 0.5
    set_max_iteration 200
    set_print_interval 10
    set_use_isomorphism 1
    start_solve
    set_dump_rounds 2
    dump_result output_result.json

This script reads one of our fixtures and emits the equivalent TexasSolver
config. The actual solve is then run via
``./bin/texassolver -i <config>``.

Parallel Rust port
==================

A separate agent has shipped a Rust port at
``crates/solver-cli/src/translate.rs`` exposed as
``solver-cli translate-fixture``. Both implementations target the same
fixture schema. This Python script still ships because:

1. The differential-testing harness at
   ``crates/solver-cli/tests/texassolver_diff.rs`` prefers it for
   Colab-only deploys where building Rust is overkill (see
   ``scripts/install-texassolver-colab.sh``).
2. It has its own doctest suite (``python3 translate_fixture.py
   --self-test``) that exercises the schema mapping independently of the
   Rust test suite — a second set of eyes on the translation logic.

Usage
=====

Direct::

    ./scripts/translate_fixture.py spot_001.json --dump output_result.json \
        > spot_001.tsconfig

From the Rust integration test, the test shells out to this script per
fixture during setup.

Tests
=====

Run the doctests::

    python3 -m doctest scripts/translate_fixture.py -v

Or invoke ``--self-test`` (same thing, friendlier output)::

    ./scripts/translate_fixture.py --self-test


Known mapping ambiguities
=========================

A few places our fixture schema does not map 1:1 to TexasSolver. Decisions
made here and why:

1. **Board format.** Our schema concatenates cards
   (``"AhKd2cQc4d"``); TexasSolver wants comma-separated
   (``"Ah,Kd,2c,Qc,4d"``). We split on every 2 chars.

2. **Pot & stack units.** Our fixtures use **chips at 1 bb = 10 chips**
   (SCHEMA.md convention, matching TexasSolver's 5/10 default). We emit
   chip counts unchanged. No scaling needed.

3. **Bet tree.** Our schema encodes this as a named preset
   (``"bet_tree": "default_v0_1"``). The translator hard-codes the
   preset's bet-size list (33/66/100 pct for flop/turn/river, plus
   all-in) in ``BET_TREE_PRESETS`` so TexasSolver sees the same tree
   our solver builds. If we ever add more presets, extend that table.

4. **Hero/villain → IP/OOP.** TexasSolver is positional (IP/OOP); our
   schema uses ``to_act`` (hero|villain) but does NOT carry position
   explicitly. We default **hero = IP** unless the optional
   ``hero_position`` field is present and says otherwise. For the
   v0.1 20-spot battery this matches the convention in SCHEMA.md's
   example (a flop BB-vs-BTN spot where hero = BTN = IP).

5. **Allin threshold.** TexasSolver's ``set_allin_threshold`` snaps bets
   above ``threshold * pot`` to all-in. 0.67 is the reference default.

6. **Iteration stopping.** Fixture ``iterations`` is a hard cap; we set
   TexasSolver's ``set_max_iteration`` to match and ``set_accuracy`` to
   a loose 0.3% so the iteration cap dominates. This gives both solvers
   the same "stop after N iterations" semantics.

7. **Thread count.** TexasSolver defaults to 1 thread in older releases.
   We default ``set_thread_num`` to ``os.cpu_count()`` so the oracle runs
   at parity with our solver. Override via the (non-schema) env-var
   ``TS_THREADS`` if you need determinism in CI.

8. **Isomorphism.** Always on (``set_use_isomorphism 1``). TexasSolver's
   iso optimization does not change strategies, only speed — any
   suspected iso-triggered bug should be investigated at our end, not
   by disabling it on the oracle side.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any, Iterable, Optional

# Defaults mirror the TexasSolver sample input so unspecified fixtures
# produce a canonical, reproducible config.
DEFAULT_ALLIN_THRESHOLD = 0.67
DEFAULT_ACCURACY = 0.3  # loose; lets iteration cap dominate
DEFAULT_PRINT_INTERVAL = 10
DEFAULT_DUMP_ROUNDS = 2

# Bet-tree presets. Key matches fixture ``input.bet_tree`` (see SCHEMA.md).
# Each preset is a dict of {street: list-of-int-pct-of-pot}.
# 'default_v0_1' matches docs/ALGORITHMS.md's bet-tree abstraction section.
BET_TREE_PRESETS: dict[str, dict[str, list[int]]] = {
    "default_v0_1": {
        "flop": [33, 66, 100],
        "turn": [50, 100, 200],
        "river": [33, 66, 100, 200],
    }
}


def _split_board(board: str) -> list[str]:
    """Split a concatenated board string into a list of 2-char cards.

    Our fixture schema stores boards as ``"AhKd2c"`` (flop),
    ``"AhKd2cQc"`` (turn), ``"AhKd2cQc4d"`` (river). Every two chars are
    one card.

    >>> _split_board("AhKd2c")
    ['Ah', 'Kd', '2c']
    >>> _split_board("AhKd2cQc4d")
    ['Ah', 'Kd', '2c', 'Qc', '4d']
    >>> _split_board("AhKd2cQc")
    ['Ah', 'Kd', '2c', 'Qc']

    Unusual lengths fail early:

    >>> _split_board("AhK")
    Traceback (most recent call last):
        ...
    ValueError: board length 3 not even: 'AhK'
    """
    if len(board) % 2 != 0:
        raise ValueError(f"board length {len(board)} not even: {board!r}")
    if len(board) not in (6, 8, 10):
        raise ValueError(
            f"board must be 3 (6 chars), 4 (8), or 5 (10) cards; got {len(board)}: {board!r}"
        )
    return [board[i : i + 2] for i in range(0, len(board), 2)]


def _streets_played(board: str) -> set[str]:
    """Determine which streets still have betting given a board string.

    The rule: flop cards visible => flop/turn/river betting. Turn card
    visible => turn/river betting. River card visible => river-only
    showdown betting.

    >>> sorted(_streets_played("AhKd2c"))
    ['flop', 'river', 'turn']
    >>> sorted(_streets_played("AhKd2cQc"))
    ['river', 'turn']
    >>> sorted(_streets_played("AhKd2cQc4d"))
    ['river']
    """
    n = len(_split_board(board))
    if n == 3:
        return {"flop", "turn", "river"}
    if n == 4:
        return {"turn", "river"}
    return {"river"}


def _format_size_list(sizes: Iterable[int]) -> str:
    """Format a bet-size list for TexasSolver.

    >>> _format_size_list([50])
    '50'
    >>> _format_size_list([33, 66, 100])
    '33,66,100'
    """
    return ",".join(str(int(round(s))) for s in sizes)


def _emit_bet_sizes(
    preset: dict[str, list[int]], street: str, streets_played: set[str]
) -> Iterable[str]:
    """Emit ``set_bet_sizes`` lines for one street, both IP and OOP.

    >>> preset = {"flop": [33, 66, 100]}
    >>> lines = list(_emit_bet_sizes(preset, "flop", {"flop"}))
    >>> lines[0]
    'set_bet_sizes oop,flop,bet,33,66,100'
    >>> any(l.endswith("oop,flop,allin") for l in lines)
    True
    >>> any(l.endswith("ip,flop,allin") for l in lines)
    True

    Skipped if street not in play:

    >>> list(_emit_bet_sizes(preset, "flop", {"river"}))
    []
    """
    if street not in streets_played:
        return

    sizes = preset.get(street, [33, 66, 100])
    size_str = _format_size_list(sizes)

    yield f"set_bet_sizes oop,{street},bet,{size_str}"
    yield f"set_bet_sizes oop,{street},raise,{size_str}"
    yield f"set_bet_sizes oop,{street},allin"
    yield f"set_bet_sizes ip,{street},bet,{size_str}"
    yield f"set_bet_sizes ip,{street},raise,{size_str}"
    yield f"set_bet_sizes ip,{street},allin"


def translate(fixture: dict[str, Any], dump_path: str = "output_result.json") -> str:
    """Translate a fixture dict into a TexasSolver config string.

    Matches the schema in
    ``crates/solver-cli/tests/fixtures/SCHEMA.md``: the spot details live
    under a nested ``input`` object; the board is a concatenated
    2-char-per-card string; pot/stack are chips at 1 bb = 10 chips.

    >>> fix = {
    ...     "id": "spot_001",
    ...     "name": "Dry AK-high flop",
    ...     "description": "Test spot",
    ...     "street": "flop",
    ...     "input": {
    ...         "board": "AhKd2c",
    ...         "hero_range": "AA, KK, AKs",
    ...         "villain_range": "QQ, AQs",
    ...         "pot": 60,
    ...         "effective_stack": 970,
    ...         "to_act": "hero",
    ...         "bet_tree": "default_v0_1",
    ...     },
    ...     "iterations": 1000,
    ...     "tolerances": {"action_freq_abs": 0.05, "ev_bb_abs": 0.1},
    ...     "expected_reference": "texassolver",
    ...     "expected_notes": "none",
    ... }
    >>> cfg = translate(fix, dump_path="out.json")
    >>> "set_pot 60" in cfg
    True
    >>> "set_effective_stack 970" in cfg
    True
    >>> "set_board Ah,Kd,2c" in cfg
    True
    >>> "set_range_ip AA, KK, AKs" in cfg
    True
    >>> "set_range_oop QQ, AQs" in cfg
    True
    >>> "build_tree" in cfg
    True
    >>> "start_solve" in cfg
    True
    >>> "dump_result out.json" in cfg
    True
    >>> "set_max_iteration 1000" in cfg
    True

    A flop fixture emits bet-sizes for flop, turn, and river:

    >>> lines = cfg.splitlines()
    >>> any(l.startswith("set_bet_sizes oop,flop") for l in lines)
    True
    >>> any(l.startswith("set_bet_sizes oop,turn") for l in lines)
    True
    >>> any(l.startswith("set_bet_sizes oop,river") for l in lines)
    True

    A river fixture (10-char board) emits river sizes only:

    >>> fix2 = json.loads(json.dumps(fix))  # deep copy
    >>> fix2["input"]["board"] = "AhKd2cQc4d"
    >>> fix2["street"] = "river"
    >>> cfg2 = translate(fix2)
    >>> lines2 = cfg2.splitlines()
    >>> any(l.startswith("set_bet_sizes oop,flop") for l in lines2)
    False
    >>> any(l.startswith("set_bet_sizes oop,turn") for l in lines2)
    False
    >>> any(l.startswith("set_bet_sizes oop,river") for l in lines2)
    True

    A villain-to-act fixture with hero=IP maps hero's range to IP:

    >>> fix3 = json.loads(json.dumps(fix))
    >>> fix3["input"]["to_act"] = "villain"
    >>> cfg3 = translate(fix3)
    >>> "set_range_ip AA, KK, AKs" in cfg3
    True
    >>> "set_range_oop QQ, AQs" in cfg3
    True

    Explicit ``hero_position: oop`` swaps IP/OOP:

    >>> fix4 = json.loads(json.dumps(fix))
    >>> fix4["input"]["hero_position"] = "oop"
    >>> cfg4 = translate(fix4)
    >>> "set_range_ip QQ, AQs" in cfg4
    True
    >>> "set_range_oop AA, KK, AKs" in cfg4
    True
    """
    if "input" not in fixture:
        raise ValueError("fixture missing required 'input' object")
    inp = fixture["input"]

    for required in (
        "board",
        "hero_range",
        "villain_range",
        "pot",
        "effective_stack",
        "bet_tree",
    ):
        if required not in inp:
            raise ValueError(f"fixture.input missing required {required!r}")

    board_raw = inp["board"]
    cards = _split_board(board_raw)
    board_ts = ",".join(cards)
    streets = _streets_played(board_raw)

    pot_chips = int(inp["pot"])
    stack_chips = int(inp["effective_stack"])

    preset_name = inp["bet_tree"]
    if preset_name not in BET_TREE_PRESETS:
        raise ValueError(
            f"unknown bet_tree preset {preset_name!r}; "
            f"known: {sorted(BET_TREE_PRESETS)}"
        )
    preset = BET_TREE_PRESETS[preset_name]

    iterations = int(fixture.get("iterations", 200))
    allin_threshold = float(fixture.get("allin_threshold", DEFAULT_ALLIN_THRESHOLD))
    accuracy = float(fixture.get("accuracy", DEFAULT_ACCURACY))
    print_interval = int(fixture.get("print_interval", DEFAULT_PRINT_INTERVAL))

    # Threading: env override > fixture override > cpu count.
    env_threads = os.environ.get("TS_THREADS")
    if env_threads:
        threads = int(env_threads)
    else:
        threads = int(fixture.get("threads", os.cpu_count() or 4))

    dump_rounds = int(fixture.get("dump_rounds", DEFAULT_DUMP_ROUNDS))

    # Hero/villain → IP/OOP mapping. Our schema does not encode position
    # directly; we default hero = IP and let fixtures opt out via a
    # non-schema ``hero_position`` field on input (string "ip" or "oop").
    hero_position = inp.get("hero_position", "ip").lower()
    if hero_position not in ("ip", "oop"):
        raise ValueError(
            f"hero_position must be 'ip' or 'oop', got {hero_position!r}"
        )
    hero_is_ip = hero_position == "ip"
    ip_range = inp["hero_range"] if hero_is_ip else inp["villain_range"]
    oop_range = inp["villain_range"] if hero_is_ip else inp["hero_range"]

    lines: list[str] = []
    lines.append("# auto-generated by scripts/translate_fixture.py")
    if "id" in fixture:
        lines.append(f"# fixture id: {fixture['id']}")
    if "name" in fixture:
        lines.append(f"# fixture name: {fixture['name']}")

    lines.append(f"set_pot {pot_chips}")
    lines.append(f"set_effective_stack {stack_chips}")
    lines.append(f"set_board {board_ts}")
    lines.append(f"set_range_ip {ip_range}")
    lines.append(f"set_range_oop {oop_range}")

    for street in ("flop", "turn", "river"):
        lines.extend(_emit_bet_sizes(preset, street, streets))

    lines.append(f"set_allin_threshold {allin_threshold}")
    lines.append("build_tree")
    lines.append(f"set_thread_num {threads}")
    lines.append(f"set_accuracy {accuracy}")
    lines.append(f"set_max_iteration {iterations}")
    lines.append(f"set_print_interval {print_interval}")
    lines.append("set_use_isomorphism 1")
    lines.append("start_solve")
    lines.append(f"set_dump_rounds {dump_rounds}")
    lines.append(f"dump_result {dump_path}")

    return "\n".join(lines) + "\n"


def _main(argv: Optional[list[str]] = None) -> int:
    ap = argparse.ArgumentParser(
        description="Translate our fixture JSON to a TexasSolver config."
    )
    ap.add_argument("fixture", nargs="?", help="path to fixture JSON")
    ap.add_argument(
        "--dump",
        default="output_result.json",
        help="path TexasSolver should write its JSON result to "
             "(passed as dump_result ...)",
    )
    ap.add_argument(
        "-o",
        "--output",
        help="write config to this file instead of stdout",
    )
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="run doctests and exit",
    )

    args = ap.parse_args(argv)

    if args.self_test:
        import doctest

        failures, tests = doctest.testmod(verbose=True)
        return 1 if failures else 0

    if not args.fixture:
        ap.error("fixture path is required unless --self-test is used")

    fixture_path = Path(args.fixture)
    fixture = json.loads(fixture_path.read_text(encoding="utf-8"))
    cfg = translate(fixture, dump_path=args.dump)

    if args.output:
        Path(args.output).write_text(cfg, encoding="utf-8")
    else:
        sys.stdout.write(cfg)

    return 0


if __name__ == "__main__":
    sys.exit(_main())
