//! A compact board representation that is efficient for simulation
use crate::compact_representation::core::CellNum as CN;
use crate::impl_common_board_traits;
use crate::types::*;
/// you almost certainly want to use the `convert_from_game` method to
/// cast from a json represention to a `CellBoard`
use crate::types::{NeighborDeterminableGame, SnakeBodyGettableGame};
use crate::wire_representation::Game;
use itertools::Itertools;
use rand::seq::SliceRandom;
use rand::Rng;
use std::borrow::Borrow;
use std::error::Error;
use std::fmt::Display;
use tracing::instrument;

use crate::{
    types::{Move, SimulableGame, SimulatorInstruments},
    wire_representation::Position,
};

use super::core::CellBoard as CCB;
use super::core::CellIndex;
use super::core::{simulate_with_moves, EvaluateMode};
use super::dimensions::{ArcadeMaze, Custom, Dimensions, Fixed, Square};

/// A compact board representation that is significantly faster for simulation than
/// `battlesnake_game_types::wire_representation::Game`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CellBoard<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> {
    embedded: CCB<T, D, BOARD_SIZE, MAX_SNAKES>,
}

impl_common_board_traits!(CellBoard);

/// 7x7 board with 4 snakes
pub type CellBoard4Snakes7x7 = CellBoard<u8, Square, { 7 * 7 }, 4>;

/// Used to represent the standard 11x11 game with up to 4 snakes.
pub type CellBoard4Snakes11x11 = CellBoard<u8, Square, { 11 * 11 }, 4>;

/// Used to represent the a 15x15 board with up to 4 snakes. This is the biggest board size that
/// can still use u8s
pub type CellBoard8Snakes15x15 = CellBoard<u8, Square, { 15 * 15 }, 8>;

/// Used to represent the largest UI Selectable board with 8 snakes.
pub type CellBoard8Snakes25x25 = CellBoard<u16, Custom, { 25 * 25 }, 8>;

/// Used to represent an absolutely silly game board
pub type CellBoard16Snakes50x50 = CellBoard<u16, Custom, { 50 * 50 }, 16>;

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    /// Builds a cellboard from a given game, will return an error if the game doesn't match
    /// the provided BOARD_SIZE or MAX_SNAKES. You are encouraged to use `CellBoard4Snakes11x11`
    /// for the common game layout
    pub fn convert_from_game(game: Game, snake_ids: &SnakeIDMap) -> Result<Self, Box<dyn Error>> {
        if game.game.ruleset.name == "wrapped" {
            return Err("Wrapped games are not supported".into());
        }

        let embedded = CCB::convert_from_game(game, snake_ids)?;
        Ok(CellBoard { embedded })
    }

    fn off_board(&self, new_head: Position) -> bool {
        new_head.x < 0
            || new_head.x >= self.embedded.get_actual_width() as i32
            || new_head.y < 0
            || new_head.y >= self.embedded.get_actual_height() as i32
    }

    /// Return an iterator over all the empty cells on the board
    pub fn get_all_empty(&self) -> impl Iterator<Item = CellIndex<T>> + '_ {
        self.embedded.get_empty_cells()
    }

    /// Spawn food deterministically using the MINSTD PRNG.
    ///
    /// Forwards to the core CellBoard's `spawn_food` implementation.
    pub fn spawn_food(
        &mut self,
        rng: &mut crate::minstd::MinstdRand,
        config: &super::core::FoodSpawnConfig,
    ) {
        self.embedded.spawn_food(rng, config);
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    RandomReasonableMovesGame for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn random_reasonable_move_for_each_snake<'a>(
        &'a self,
        rng: &'a mut impl Rng,
    ) -> Box<dyn std::iter::Iterator<Item = (SnakeId, Move)> + 'a> {
        Box::new(
            self.reasonable_moves_for_each_snake()
                .map(move |(sid, mvs)| (sid, *mvs.choose(rng).unwrap())),
        )
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize> ReasonableMovesGame
    for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn reasonable_moves_for_each_snake(
        &self,
    ) -> Box<dyn std::iter::Iterator<Item = (SnakeId, Vec<Move>)> + '_> {
        let width = self.embedded.get_actual_width();
        Box::new(
            self.embedded
                .iter_healths()
                .enumerate()
                .filter(|(_, health)| **health > 0)
                .map(move |(idx, _)| {
                    let head_pos = self.get_head_as_position(&SnakeId(idx as u8));

                    let mvs = IntoIterator::into_iter(Move::all())
                        .filter(|mv| {
                            let new_head = head_pos.add_vec(mv.to_vector());
                            let ci = CellIndex::new(new_head, width);

                            !self.off_board(new_head)
                                && (!self.embedded.cell_is_body(ci)
                                    || self.embedded.cell_is_single_tail(ci))
                                && !self.embedded.cell_is_snake_head(ci)
                        })
                        .collect_vec();
                    let mvs = if mvs.is_empty() { vec![Move::Up] } else { mvs };

                    (SnakeId(idx as u8), mvs)
                }),
        )
    }
}

impl<
        T: SimulatorInstruments,
        D: Dimensions,
        N: CN,
        const BOARD_SIZE: usize,
        const MAX_SNAKES: usize,
    > SimulableGame<T, MAX_SNAKES> for CellBoard<N, D, BOARD_SIZE, MAX_SNAKES>
{
    #[allow(clippy::type_complexity)]
    #[instrument(level = "trace", skip_all)]
    fn simulate_with_moves<S>(
        &self,
        instruments: &T,
        snake_ids_and_moves: impl IntoIterator<Item = (Self::SnakeIDType, S)>,
    ) -> Box<dyn Iterator<Item = (Action<MAX_SNAKES>, Self)> + '_>
    where
        S: Borrow<[Move]>,
    {
        Box::new(
            simulate_with_moves(
                &self.embedded,
                instruments,
                snake_ids_and_moves,
                EvaluateMode::Standard,
            )
            .map(|v| {
                let (action, board) = v;
                (action, Self { embedded: board })
            }),
        )
    }
}

impl<T: CN, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    NeighborDeterminableGame for CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    fn possible_moves<'a>(
        &'a self,
        pos: &Self::NativePositionType,
    ) -> Box<dyn std::iter::Iterator<Item = (Move, CellIndex<T>)> + 'a> {
        let width = self.embedded.get_actual_width();
        let head_pos = pos.into_position(width);

        Box::new(
            Move::all_iter()
                .map(move |mv| {
                    let new_head = head_pos.add_vec(mv.to_vector());
                    let ci = CellIndex::new(new_head, width);

                    (mv, new_head, ci)
                })
                .filter(move |(_mv, new_head, _)| !self.off_board(*new_head))
                .map(|(mv, _, ci)| (mv, ci)),
        )
    }

    fn neighbors<'a>(
        &'a self,
        pos: &Self::NativePositionType,
    ) -> Box<dyn Iterator<Item = CellIndex<T>> + 'a> {
        let width = self.embedded.get_actual_width();
        let head_pos = pos.into_position(width);

        Box::new(
            Move::all_iter()
                .map(move |mv| {
                    let new_head = head_pos.add_vec(mv.to_vector());
                    let ci = CellIndex::new(new_head, width);

                    (new_head, ci)
                })
                .filter(move |(new_head, _)| !self.off_board(*new_head))
                .map(|(_, ci)| ci),
        )
    }
}

/// Enum that holds a Cell Board sized right for the given game
#[derive(Debug)]
pub enum BestCellBoard {
    /// A game that can have a max height and width of 7x7 and 4 snakes
    Tiny(Box<CellBoard4Snakes7x7>),
    /// A exactly 7x7 board with 4 snakes
    SmallExact(Box<CellBoard<u8, Fixed<7, 7>, { 7 * 7 }, 4>>),
    /// A game that can have a max height and width of 11x11 and 4 snakes
    Standard(Box<CellBoard4Snakes11x11>),
    /// A exactly 11x11 board with 4 snakes
    MediumExact(Box<CellBoard<u8, Fixed<11, 11>, { 11 * 11 }, 4>>),
    /// A game that can have a max height and width of 15x15 and 4 snakes
    LargestU8(Box<CellBoard8Snakes15x15>),
    /// A exactly 19x19 board with 4 snakes
    LargeExact(Box<CellBoard<u16, Fixed<19, 19>, { 19 * 19 }, 4>>),
    /// A board that fits the Arcade Maze map
    ArcadeMaze(Box<CellBoard<u16, ArcadeMaze, { 19 * 21 }, 4>>),
    /// A board that fits the Arcade Maze map
    ArcadeMaze8Snake(Box<CellBoard<u16, ArcadeMaze, { 19 * 21 }, 8>>),
    /// A game that can have a max height and width of 25x25 and 8 snakes
    Large(Box<CellBoard8Snakes25x25>),
    /// A game that can have a max height and width of 50x50 and 16 snakes
    Silly(Box<CellBoard16Snakes50x50>),
}

/// Trait to get the best sized cellboard for the given game. It returns the smallest Compact board
/// that has enough room to fit the given Wire game. If the game can't fit in any of our Compact
/// boards we panic. However the largest board available is MUCH larger than the biggest selectable
/// board in the Battlesnake UI
pub trait ToBestCellBoard {
    #[allow(missing_docs)]
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>>;
}

impl ToBestCellBoard for Game {
    fn to_best_cell_board(self) -> Result<BestCellBoard, Box<dyn Error>> {
        let width = self.board.width;
        let height = self.board.height;
        let num_snakes = self.board.snakes.len();
        let id_map = build_snake_id_map(&self);

        let best_board = if width == 7 && height == 7 && num_snakes <= 4 {
            BestCellBoard::SmallExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 7 && height <= 7 && num_snakes <= 4 {
            BestCellBoard::Tiny(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 11 && height == 11 && num_snakes <= 4 {
            BestCellBoard::MediumExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 11 && height <= 11 && num_snakes <= 4 {
            BestCellBoard::Standard(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 15 && height <= 15 && num_snakes <= 8 {
            BestCellBoard::LargestU8(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 19 && num_snakes <= 4 {
            BestCellBoard::LargeExact(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 21 && num_snakes <= 4 {
            BestCellBoard::ArcadeMaze(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width == 19 && height == 21 && num_snakes <= 8 {
            BestCellBoard::ArcadeMaze8Snake(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 25 && height < 25 && num_snakes <= 8 {
            BestCellBoard::Large(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else if width <= 50 && height <= 50 && num_snakes <= 16 {
            BestCellBoard::Silly(Box::new(CellBoard::convert_from_game(self, &id_map)?))
        } else {
            panic!("No board was big enough")
        };

        Ok(best_board)
    }
}

#[cfg(test)]
mod test {

    use itertools::Itertools;

    use super::*;
    use crate::{
        compact_representation::core::Cell, game_fixture, types::build_snake_id_map,
        wire_representation::Game as DEGame,
    };
    #[derive(Debug)]
    struct Instruments;
    impl SimulatorInstruments for Instruments {
        fn observe_simulation(&self, _: std::time::Duration) {}
    }

    #[test]
    fn test_compact_board_conversion() {
        let start_of_game_fixture =
            game_fixture(include_str!("../../../fixtures/start_of_game.json"));
        let converted = Game::to_best_cell_board(start_of_game_fixture);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::MediumExact(_) => {}
            _ => panic!("expected standard board"),
        }

        let tiny_board = game_fixture(include_str!("../../../fixtures/7x7board.json"));
        let converted = Game::to_best_cell_board(tiny_board);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::SmallExact(_) => {}
            _ => panic!("expected standard board"),
        }

        let non_standard_small_board =
            game_fixture(include_str!("../../../fixtures/8x8board.json"));
        let converted = Game::to_best_cell_board(non_standard_small_board);
        assert!(converted.is_ok());
        let u = converted.unwrap();
        match u {
            BestCellBoard::Standard(_) => {}
            _ => panic!("expected standard board"),
        }
    }

    #[test]
    fn test_head_gettable() {
        let game_fixture = include_str!("../../../fixtures/late_stage.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        assert_eq!(
            compact.get_head_as_position(&SnakeId(0)),
            Position { x: 4, y: 6 }
        );
        assert_eq!(
            compact.get_head_as_native_position(&SnakeId(0)),
            CellIndex(6 * 11 + 4)
        );
    }

    #[test]
    fn test_triple_stacked_eats_food() {
        // Regression test for the engine-verifier "Bug A" finding (2026-05-01):
        // a length-3 fully-stacked snake that eats food on its first move
        // should end up at length 4 with three body segments stacked at the
        // original position. The fix introduces the `BODY_TRIPLE_STACKED`
        // cell kind for the post-eat shape; see `DESIGN_stacked_food_fix.md`.
        let game_fixture = include_str!("../../../fixtures/triple_stacked_with_food_ahead.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let instruments = Instruments;
        let res = compact
            .simulate_with_moves(&instruments, vec![(SnakeId(0), [Move::Right].as_slice())])
            .collect_vec();
        let after = res[0].1;

        // Body should be: head at (9,2), then three stacked tail segments at (8,2)
        let body = after.get_snake_body_vec(&SnakeId(0));
        let head_at_9_2 = CellIndex::<u8>::new(Position { x: 9, y: 2 }, 11);
        let tail_at_8_2 = CellIndex::<u8>::new(Position { x: 8, y: 2 }, 11);
        assert_eq!(
            body,
            vec![head_at_9_2, tail_at_8_2, tail_at_8_2, tail_at_8_2],
            "expected length-4 body with triple-stacked tail at (8,2)"
        );
        assert_eq!(after.get_length(&SnakeId(0)), 4);
        assert_eq!(after.get_health(&SnakeId(0)), 100);
    }

    #[test]
    fn test_body_triple_stacked_round_trips() {
        // The post-eat shape `[head, x, x, x]` (where the tail is three
        // segments stacked at a single non-head cell) is a legitimate
        // mid-game body. Convert it through `convert_from_game` and back
        // via `get_snake_body_vec`; the body must round-trip exactly.
        let game_fixture = include_str!("../../../fixtures/post_eat_body_triple_stacked.json");
        let g: DEGame = serde_json::from_slice(game_fixture.as_bytes())
            .expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let body = compact.get_snake_body_vec(&SnakeId(0));
        let head = CellIndex::<u8>::new(Position { x: 9, y: 2 }, 11);
        let tail = CellIndex::<u8>::new(Position { x: 8, y: 2 }, 11);
        assert_eq!(
            body,
            vec![head, tail, tail, tail],
            "post-eat body must round-trip through the compact representation"
        );
        assert_eq!(compact.get_length(&SnakeId(0)), 4);
    }

    #[test]
    fn test_body_triple_stacked_shrinks_on_next_move() {
        // After the post-eat shape, advance one more step (no food). The
        // BODY_TRIPLE_STACKED tail cell must demote to DOUBLE_STACKED,
        // length stays 4, and the body shape becomes `[new_head, old_head, x, x]`.
        let game_fixture = include_str!("../../../fixtures/post_eat_body_triple_stacked.json");
        let g: DEGame = serde_json::from_slice(game_fixture.as_bytes())
            .expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let instruments = Instruments;
        let res = compact
            .simulate_with_moves(&instruments, vec![(SnakeId(0), [Move::Up].as_slice())])
            .collect_vec();
        let after = res[0].1;

        let body = after.get_snake_body_vec(&SnakeId(0));
        let new_head = CellIndex::<u8>::new(Position { x: 9, y: 3 }, 11);
        let old_head = CellIndex::<u8>::new(Position { x: 9, y: 2 }, 11);
        let stack = CellIndex::<u8>::new(Position { x: 8, y: 2 }, 11);
        assert_eq!(
            body,
            vec![new_head, old_head, stack, stack],
            "next move after eat-while-stacked must shrink the triple-stack to a double-stack"
        );
        assert_eq!(after.get_length(&SnakeId(0)), 4);
    }

    #[test]
    fn test_snake_collision_ordering() {
        // Regression test for engine-verifier "Bug B" (2026-05-01).
        //
        // Setup (turn 5):
        //   snake-0 body=[(3,6),(3,7),(4,7)], move=Right -> new_head=(4,6)
        //   snake-1 body=[(4,6),(5,6),(6,6)], move=Right -> new_head=(5,6)
        //
        // Expected per Battlesnake spec (all moves resolve simultaneously):
        //   snake-0 dies: new_head (4,6) collides with snake-1's post-move
        //                 body which still occupies (4,6) as a body segment.
        //   snake-1 dies: self-collision -- new_head (5,6) is its own neck.
        //
        // Previous Rust behavior incorrectly produced snake-0 ALIVE because
        // snake-1's `Dead` move-result triggered `kill_and_remove` inside the
        // first move-processing loop, wiping snake-1's body before snake-0's
        // collision check could see it.
        let game_fixture = include_str!("../../../fixtures/snake_collision_ordering.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let instruments = Instruments;
        let res = compact
            .simulate_with_moves(
                &instruments,
                vec![
                    (SnakeId(0), [Move::Right].as_slice()),
                    (SnakeId(1), [Move::Right].as_slice()),
                ],
            )
            .collect_vec();
        assert_eq!(res.len(), 1, "exactly one move combination");
        let after = res[0].1;

        assert_eq!(
            after.get_health(&SnakeId(0)),
            0,
            "snake-0 must die from collision with snake-1's body at (4,6)"
        );
        assert_eq!(
            after.get_health(&SnakeId(1)),
            0,
            "snake-1 must die from self-collision into its neck at (5,6)"
        );
    }

    #[test]
    fn test_tail_collision() {
        let game_fixture = include_str!("../../../fixtures/start_of_game.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let mut compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();
        let moves = [
            Move::Left,
            Move::Down,
            Move::Right,
            Move::Up,
            Move::Left,
            Move::Down,
        ];
        let instruments = Instruments;
        eprintln!("{compact}");
        for mv in moves {
            let res = compact
                .simulate_with_moves(&instruments, vec![(SnakeId(0), [mv].as_slice())])
                .collect_vec();
            compact = res[0].1;
            eprintln!("{compact}");
        }
        assert!(compact.get_health(&SnakeId(0)) > 0);
    }

    #[test]
    fn test_set_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
    }

    #[test]
    fn test_clear_hazard() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        assert!(c.is_food());
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_food());
        assert!(!c.is_hazard());
        assert!(!c.is_head());
        assert!(!c.is_body());
        let mut c: Cell<u8> = Cell::make_double_stacked_piece(SnakeId(0), CellIndex(0));
        c.set_hazard();
        c.clear_hazard();
        assert!(c.is_body());
        assert!(!c.is_hazard());
    }

    #[test]
    fn test_remove() {
        let mut c: Cell<u8> = Cell::make_body_piece(SnakeId(3), CellIndex(17));
        c.remove();
        c.set_hazard();
        assert!(c.is_empty());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }
    #[test]
    fn test_set_food() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_food();
        c.set_hazard();
        assert!(c.is_food());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().is_none());
        assert!(c.get_idx() == CellIndex(0));
    }

    #[test]
    fn test_set_head() {
        let mut c: Cell<u8> = Cell::empty();
        c.set_head(SnakeId(3), CellIndex(17));
        c.set_hazard();
        assert!(c.is_head());
        assert!(c.is_hazard());
        assert!(c.get_snake_id().unwrap() == SnakeId(3));
        assert!(c.get_idx() == CellIndex(17));
    }

    #[test]
    fn test_food_queryable() {
        let game_fixture = include_str!("../../../fixtures/late_stage.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        assert!(!compact.is_food(&CellIndex(6 * 11 + 4)));

        assert!(compact.is_food(&CellIndex(2 * 11)));
        assert!(compact.is_food(&CellIndex(9 * 11)));
        assert!(compact.is_food(&CellIndex(3 * 11 + 4)));
    }

    #[test]
    fn test_neighbors_and_possible_moves_start_of_game() {
        let game_fixture = include_str!("../../../fixtures/start_of_game.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(8 * 11 + 5));

        let expected_possible_moves = vec![
            (Move::Up, CellIndex(9 * 11 + 5)),
            (Move::Down, CellIndex(7 * 11 + 5)),
            (Move::Left, CellIndex(8 * 11 + 4)),
            (Move::Right, CellIndex(8 * 11 + 6)),
        ];

        assert_eq!(
            compact.possible_moves(&head).collect::<Vec<_>>(),
            expected_possible_moves
        );

        assert_eq!(
            compact.neighbors(&head).collect::<Vec<_>>(),
            expected_possible_moves
                .into_iter()
                .map(|(_, pos)| pos)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_neighbors_and_possible_moves_cornered() {
        let game_fixture = include_str!("../../../fixtures/cornered.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(10 * 11));

        let expected_possible_moves = vec![
            (Move::Down, CellIndex(9 * 11)),
            (Move::Right, CellIndex(10 * 11 + 1)),
        ];

        assert_eq!(
            compact.possible_moves(&head).collect::<Vec<_>>(),
            expected_possible_moves
        );

        assert_eq!(
            compact.neighbors(&head).collect::<Vec<_>>(),
            expected_possible_moves
                .into_iter()
                .map(|(_, pos)| pos)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_tail_chase() {
        let game_fixture = include_str!("../../../fixtures/tail_chase.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let head = compact.get_head_as_native_position(&SnakeId(0));
        assert_eq!(head, CellIndex(0));

        let mut reasonable_moves = compact.reasonable_moves_for_each_snake();
        let reasonable_moves_for_me = reasonable_moves.next().unwrap().1;

        assert_eq!(reasonable_moves_for_me, vec![Move::Up]);
    }

    #[test]
    fn test_simulate_stacked_hazards() {
        let game_fixture = include_str!("../../../fixtures/stacked_hazards.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let moves = vec![(SnakeId(0), vec![Move::Down])];

        let result = compact
            .simulate_with_moves(&Instruments {}, moves)
            .next()
            .unwrap()
            .1;

        assert_eq!(result.get_health(&SnakeId(0)), 69);
    }

    #[test]
    fn test_simulate_stacked_hazards_no_hazard_on_sqaure() {
        let game_fixture = include_str!("../../../fixtures/stacked_hazards.json");
        let g: Result<DEGame, _> = serde_json::from_slice(game_fixture.as_bytes());
        let g = g.expect("the json literal is valid");
        let snake_id_mapping = build_snake_id_map(&g);
        let compact: CellBoard4Snakes11x11 = g.as_cell_board(&snake_id_mapping).unwrap();

        let moves = vec![(SnakeId(0), vec![Move::Right])];

        let result = compact
            .simulate_with_moves(&Instruments {}, moves)
            .next()
            .unwrap()
            .1;

        assert_eq!(result.get_health(&SnakeId(0)), 99);
    }
}
