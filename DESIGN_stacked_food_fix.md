# Design: Triple-stacked snake eating food (Bug A)

> Companion to the failing-but-`#[ignore]`'d regression test in
> `src/compact_representation/standard/mod.rs::test_triple_stacked_eats_food`
> and to `byte-scratch:engine-verifier/FAILURES_ANALYSIS.md` Bug A.

## Problem (recap)

When a fully triple-stacked snake (start-of-game body
`[(x,y), (x,y), (x,y)]`) moves into a food cell, the post-move body is
`[new_head, old_cell, old_cell, old_cell]` — length 4, with three segments
at the original cell.

The compact representation has no encoding for that shape:

- `TRIPLE_STACKED_PIECE` is implicitly the head (`is_head()` returns true,
  `get_next_index()` returns `None` because the head's next is itself —
  there's no chain pointer back to a separate head cell).
- `convert_from_game` rejects bodies of this shape outright at
  `cell_board/mod.rs:263–265` ("bad body stack: 3 segs on same square and
  more than one unique position").
- The eval path (`eval.rs:364–376`, the `is_triple_stacked_piece()` branch
  on the old-head cell) demotes to double-stacked unconditionally, losing
  one stack level. There is a `FIXME` comment marking the exact site.

## Proposed fix: new cell kind `BODY_TRIPLE_STACKED`

The KIND field is a 3-bit subfield of `flags` (`KIND_MASK = 0x07`).
Currently used values: `0x01 BODY`, `0x02 DOUBLE`, `0x03 TRIPLE`,
`0x04 FOOD`, `0x05 EMPTY`, `0x06 HEAD`. **`0x00` and `0x07` are free.**

Add:

```rust
const BODY_TRIPLE_STACKED_PIECE: u8 = 0x07;
```

Semantics:
- `is_body_segment` → **true**
- `is_head` → **false** (key difference from existing `TRIPLE_STACKED_PIECE`)
- `is_stacked` → **true**
- `get_next_index` → **`Some(self.idx)`** (carries chain pointer like
  double-stacked does)
- `get_snake_id` → **`Some(self.id)`**

Constructor mirrors `make_double_stacked_piece`:

```rust
pub fn make_body_triple_stacked_piece(sid: SnakeId, next_index: CellIndex<T>) -> Self {
    Cell {
        flags: BODY_TRIPLE_STACKED_PIECE,
        id: sid,
        idx: next_index,
        hazard_count: 0,
    }
}

pub fn set_body_triple_stacked(&mut self, sid: SnakeId, next_pos: CellIndex<T>) {
    self.flags = (self.flags & !KIND_MASK) | BODY_TRIPLE_STACKED_PIECE;
    self.id = sid;
    self.idx = next_pos;
}
```

Plus a `set_cell_body_triple_stacked` helper on `CellBoard` matching the
existing `set_cell_double_stacked` pattern.

## Why distinguish from existing `TRIPLE_STACKED_PIECE`?

The existing kind doubles as "this cell is the head of a fully-stacked
snake (the head, neck, and tail all live here)." It has no chain pointer
because there is nowhere else to chain *to*. We can't reuse it for the
post-eat shape because:

1. After the move, the head is on a different cell — the old cell must
   not report `is_head() == true`, or `set_cell_head(new_head, …)` will
   wreck head invariants and `heads[id]` will disagree with the board.
2. The old cell needs a chain pointer to `new_head` so
   `get_snake_body_vec` and `get_next_index` can walk tail→head.

Keeping the original `TRIPLE_STACKED_PIECE` semantics intact (it remains
the start-of-game self-referential cell) avoids touching every reader
that assumes "triple-stack ⇒ head".

## Touch sites

Only files under `src/compact_representation/core/` plus a couple of
fixture/test additions.

### `core/mod.rs`

1. Add `BODY_TRIPLE_STACKED_PIECE` constant.
2. Add `make_body_triple_stacked_piece` constructor.
3. Add `set_body_triple_stacked` setter.
4. Add `is_body_triple_stacked_piece` predicate.
5. Extend predicates:
   - `is_body_segment` → also true for new kind
   - `is_body` → also true for new kind
   - `is_stacked` → also true for new kind
   - `get_next_index` → `Some(self.idx)` for new kind (alongside body
     piece and double-stack)
   - `get_snake_id` (already covered if we extend `is_body_segment`,
     since it gates on `is_body_segment() || is_head()`)

### `core/cell_board/mod.rs`

1. **Relax `convert_from_game` rejection** at line 263. The check
   `counts.values().any(|v| *v == TRIPLE_STACK) && counts.len() != 1`
   was a guard against this exact shape; now it's a legal mid-game state.
   Remove the early return.
2. **Map count==TRIPLE for non-head cells to the new kind** in the
   loop at line 298. The current branch is:
   ```rust
   if *count == TRIPLE_STACK {
       Cell::make_triple_stacked_piece(snake_id)
   }
   ```
   Becomes:
   ```rust
   if *count == TRIPLE_STACK {
       if *pos == snake.head {
           // Start-of-game self-stacked snake: no other unique cells.
           Cell::make_triple_stacked_piece(snake_id)
       } else {
           Cell::make_body_triple_stacked_piece(snake_id, next_index)
       }
   }
   ```
   `next_index` is already maintained correctly by the existing loop —
   it points to the previous (closer-to-head) unique cell.
3. Add `set_cell_body_triple_stacked` helper alongside
   `set_cell_double_stacked`.
4. Extend the `is_body_segment` cluster around line 443 if any local
   predicate enumerates the kinds explicitly (the `is_body_segment`
   method on `Cell` is the primary check).

### `core/cell_board/eval.rs`

Replace the `FIXME` branch at lines 363–379:

```rust
let old_head_cell = self.get_cell(old_head);
if old_head_cell.is_triple_stacked_piece() {
    if ate_food {
        // Body becomes [new_head, old_head x3].
        // Old head cell holds 3 segments and chains to new_head.
        new.set_cell_body_triple_stacked(old_head, id, new_head);
    } else {
        // Body becomes [new_head, old_head x2].
        new.set_cell_double_stacked(old_head, id, new_head);
    }
} else {
    new.set_cell_body_piece(old_head, id, new_head);
}
```

The non-`ate_food` arm matches today's behavior; the `ate_food` arm is
the actual fix.

**Tail-removal check** (line 206 area): the `old_tail_cell.is_double_stacked_piece()`
branch demotes a double-stack to a body piece on tail removal. The new
`BODY_TRIPLE_STACKED` cell needs the same shrink-on-tail path —
demote to `DOUBLE_STACKED_PIECE` (still has chain pointer to next index):

```rust
if old_tail_cell.is_double_stacked_piece() {
    new.set_cell_body_piece(old_tail, id, old_tail_cell.get_idx());
} else if old_tail_cell.is_body_triple_stacked_piece() {
    new.set_cell_double_stacked(old_tail, id, old_tail_cell.get_idx());
} else {
    new.cell_remove(old_tail);
    new.set_cell_head(old_head, id, new_tail);
}
```

Note: when the tail is a `BODY_TRIPLE_STACKED` cell, the head is *not*
on this cell (that's the whole point of the new kind), so the `else`
arm's `set_cell_head(old_head, …)` is correctly skipped.

### `core/cell_board/snake_body_gettable.rs`

`get_snake_body_vec` walks tail→head and pushes once per segment, with
extra pushes for stacked kinds. Add the new kind:

```rust
if self.get_cell(c).is_body_triple_stacked_piece() {
    body.push(c);
    body.push(c);
}
```

`get_snake_body_iter` walks unique positions only — it relies on
`get_next_index()`. Since `BODY_TRIPLE_STACKED.get_next_index()` returns
`Some(self.idx)`, the iter walks correctly without changes.

### Other readers

Search hits to verify there are no other special-cases:
`is_double_stacked_piece` and `is_triple_stacked_piece` references are
all in `core/`. Anywhere code uses `is_body_segment` / `is_body` /
`is_stacked` / `get_next_index` / `get_snake_id` will be correct
automatically.

## Tests

1. **Un-`#[ignore]`** the existing `test_triple_stacked_eats_food`. It
   should pass after the fix.
2. **Add round-trip test:** convert a Game with `body = [(0,0),(0,0),(0,0),(1,0)]`
   (4 segments, 3 stacked at non-head, 1 at head) into a CellBoard and
   back; assert `get_snake_body_vec` returns the same body.
3. **Add eat-from-body-triple test:** advance the post-eat snake one
   more step (no food) and confirm the body shrinks correctly:
   `[new_head_2, new_head, old x2]` — i.e., the `BODY_TRIPLE_STACKED`
   cell demotes to `DOUBLE_STACKED` on the next tail removal.
4. **Re-run engine-verifier** (`byte-scratch/engine-verifier`) and
   confirm Bug A's 13 `fail_*.json` cases (`fail_103, 104, 112, 135,
   137, 141, 39, 44, 55, 83, fail_multi_15, fail_multi_31, fail_multi_40`)
   all resolve.

## Risk / size

- ~60 lines of changes across 3–4 files in `core/`.
- No public-API surface change; only internal cell encoding adds a kind.
- The relaxation in `convert_from_game` makes a previously-rejected
  input shape legal — that's a behavior change, but it's the bug fix.
- Existing `TRIPLE_STACKED_PIECE` semantics are preserved; the change
  is purely additive at the kind level.

## Open questions

1. **Naming.** `BODY_TRIPLE_STACKED_PIECE` is descriptive but long.
   Alternatives: `STACKED_3_BODY`, `DEEP_STACKED_BODY`. Defer to repo
   convention.
2. **Quad-stack?** Theoretically a snake could be 4-stacked (eat twice
   in two turns while fully stacked at the same cell). Battlesnake
   rules don't permit this in practice — eating moves the head off the
   stack — so we don't need a `BODY_QUAD_STACKED` kind. Worth a comment
   in `eval.rs` though.
3. **Migrate `TRIPLE_STACKED_PIECE` away from being implicit head?**
   Cleaner long-term to make all stacked kinds non-head and require an
   explicit `SNAKE_HEAD` cell at `head_idx`. Larger blast radius
   (touches `is_head`, `get_tail_position`, `convert_from_game` head
   handling). Out of scope for this fix; could be a follow-up.
