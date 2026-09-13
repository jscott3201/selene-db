"""Small, finite path-binding selector reference; not a GQL interpreter.

The caller supplies already-valid candidate bindings, including their multiplicity.
This module does not check adjacency, parse patterns, implement nested mode scopes,
or perform reduced-match deduplication. See README.md for its deliberate limits.
"""
from __future__ import annotations

from collections import defaultdict
from dataclasses import dataclass
from typing import Callable, Iterable, Literal

Mode = Literal["walk", "trail", "simple", "acyclic"]
Selection = Literal["any", "all_shortest", "counted_shortest", "shortest_groups"]


@dataclass(frozen=True)
class PathBinding:
    nodes: tuple[int, ...]
    edges: tuple[int, ...]
    binding_tag: str = ""

    def __post_init__(self) -> None:
        if not self.nodes or len(self.nodes) != len(self.edges) + 1:
            raise ValueError("A path needs one more node than edges, including a zero-edge path.")

    @property
    def length(self) -> int:
        return len(self.edges)

    @property
    def endpoints(self) -> tuple[int, int]:
        return self.nodes[0], self.nodes[-1]


def admits_mode(path: PathBinding, mode: Mode) -> bool:
    """Check one whole-path mode; local/nested scopes are intentionally not modeled."""
    if mode == "walk":
        return True
    if mode == "trail":
        return len(set(path.edges)) == len(path.edges)
    if mode == "acyclic":
        return len(set(path.nodes)) == len(path.nodes)
    if mode == "simple":
        prefix = path.nodes[:-1]
        return len(set(prefix)) == len(prefix) and (
            path.nodes[-1] not in prefix or path.nodes[-1] == path.nodes[0]
        )
    raise ValueError(f"Unknown mode: {mode}")


def select_paths(
    paths: Iterable[PathBinding],
    selection: Selection,
    *,
    count: int | None = None,
    qualifies: Callable[[PathBinding], bool] | None = None,
) -> list[PathBinding]:
    """Qualify -> partition by endpoints -> select, over an explicitly finite input.

    Tie-breaking is deterministic for this reference. It is not a promise about
    portable GQL row order. Qualifies supplies the already-evaluated acceptance
    condition; this module does not implement GQL three-valued expression logic.
    """
    allowed = {"any", "all_shortest", "counted_shortest", "shortest_groups"}
    if selection not in allowed:
        raise ValueError(f"Unknown selection: {selection}")
    if selection == "all_shortest":
        if count is not None:
            raise ValueError("all_shortest does not accept count")
    elif type(count) is not int or count < 0:
        raise ValueError("Count must be a non-negative integer")

    partitions: dict[tuple[int, int], list[PathBinding]] = defaultdict(list)
    for path in paths:
        if qualifies is None or qualifies(path):
            partitions[path.endpoints].append(path)

    result: list[PathBinding] = []
    for endpoints in sorted(partitions):
        group = sorted(
            partitions[endpoints],
            key=lambda p: (p.length, p.nodes, p.edges, p.binding_tag),
        )
        if selection == "all_shortest":
            result.extend(p for p in group if p.length == group[0].length)
        elif selection in {"any", "counted_shortest"}:
            # Choosing shortest first is one legal ANY policy for finite input.
            result.extend(group[:count])
        else:
            lengths = sorted({p.length for p in group})[:count]
            selected_lengths = set(lengths)
            result.extend(p for p in group if p.length in selected_lengths)
    return result
