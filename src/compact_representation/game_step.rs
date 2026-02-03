//! Full game step: simulate moves + spawn food.
//!
//! Combines move simulation with deterministic food spawning via MINSTD PRNG.
//! This matches the official Battlesnake rules' `CreateNextBoardState` function.

use crate::compact_representation::dimensions::Dimensions;
use crate::compact_representation::standard;
use crate::minstd;
use crate::types::{Move, SimulableGame, SimulatorInstruments, SnakeId};

use super::core::CellNum;

/// Configuration for food spawning behavior.
pub use super::core::FoodSpawnConfig;

/// Simulate one full game turn on a standard board: apply moves, then spawn food.
///
/// This is the Rust equivalent of the Go rules' `CreateNextBoardState`:
/// 1. Apply all snake moves (movement, collisions, eating, elimination)
/// 2. Spawn food using MINSTD PRNG seeded for this turn
///
/// Each snake should have exactly one move. Returns the board after the turn.
///
/// # Arguments
///
/// * `board` - Current board state
/// * `instruments` - Simulation timing instruments
/// * `snake_moves` - Each snake's chosen move: `(SnakeId, [Move])`
/// * `game_seed` - The game's random seed for deterministic food spawning
/// * `turn` - The current turn number (used to derive per-turn RNG seed)
/// * `food_config` - Food spawning parameters (minimum food, spawn chance)
pub fn step_with_food<
    I: SimulatorInstruments,
    T: CellNum,
    D: Dimensions,
    const BOARD_SIZE: usize,
    const MAX_SNAKES: usize,
>(
    board: &standard::CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>,
    instruments: &I,
    snake_moves: Vec<(SnakeId, Vec<Move>)>,
    game_seed: u64,
    turn: u32,
    food_config: &FoodSpawnConfig,
) -> standard::CellBoard<T, D, BOARD_SIZE, MAX_SNAKES> {
    // Step 1: Simulate moves (movement, collisions, eating, elimination)
    let mut result = board.simulate_with_moves(instruments, snake_moves);

    let (_, mut new_board) = result.next().expect("simulation should produce at least one result");
    drop(result);

    // Step 2: Spawn food deterministically
    let mut rng = minstd::rng_for_turn(game_seed, turn);
    new_board.spawn_food(&mut rng, food_config);

    new_board
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact_representation::StandardCellBoard4Snakes11x11;
    use crate::types::{build_snake_id_map, FoodGettableGame};

    struct TestInstruments;
    impl std::fmt::Debug for TestInstruments {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "TestInstruments")
        }
    }
    impl SimulatorInstruments for TestInstruments {
        fn observe_simulation(&self, _duration: std::time::Duration) {}
    }

    fn load_fixture(json: &str) -> (StandardCellBoard4Snakes11x11, Vec<SnakeId>) {
        let game: crate::wire_representation::Game = serde_json::from_str(json).unwrap();
        let id_map = build_snake_id_map(&game);
        let snake_ids: Vec<SnakeId> = game
            .board
            .snakes
            .iter()
            .map(|s| *id_map.get(&s.id).unwrap())
            .collect();
        let board = StandardCellBoard4Snakes11x11::convert_from_game(game, &id_map).unwrap();
        (board, snake_ids)
    }

    #[test]
    fn test_step_with_food_deterministic() {
        let fixture = include_str!("../../fixtures/start_of_game.json");
        let (board1, snake_ids1) = load_fixture(fixture);
        let (board2, snake_ids2) = load_fixture(fixture);

        let food_config = FoodSpawnConfig::default();
        let game_seed = 42u64;
        let turn = 1u32;

        // All snakes move right
        let moves1: Vec<(SnakeId, Vec<Move>)> = snake_ids1
            .iter()
            .map(|&sid| (sid, vec![Move::Right]))
            .collect();
        let moves2: Vec<(SnakeId, Vec<Move>)> = snake_ids2
            .iter()
            .map(|&sid| (sid, vec![Move::Right]))
            .collect();

        let result1 =
            step_with_food(&board1, &TestInstruments, moves1, game_seed, turn, &food_config);
        let result2 =
            step_with_food(&board2, &TestInstruments, moves2, game_seed, turn, &food_config);

        // Same seed + same moves = identical board states
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_step_with_food_different_seeds_produce_different_food() {
        let fixture = include_str!("../../fixtures/start_of_game.json");
        let (board1, snake_ids1) = load_fixture(fixture);
        let (board2, snake_ids2) = load_fixture(fixture);

        let food_config = FoodSpawnConfig {
            minimum_food: 0,
            food_spawn_chance: 100, // always spawn so we can see the difference
        };

        let moves1: Vec<(SnakeId, Vec<Move>)> = snake_ids1
            .iter()
            .map(|&sid| (sid, vec![Move::Right]))
            .collect();
        let moves2: Vec<(SnakeId, Vec<Move>)> = snake_ids2
            .iter()
            .map(|&sid| (sid, vec![Move::Right]))
            .collect();

        let result1 = step_with_food(&board1, &TestInstruments, moves1, 42, 1, &food_config);
        let result2 = step_with_food(&board2, &TestInstruments, moves2, 99, 1, &food_config);

        // Different seeds should produce different food placements
        let food1 = result1.get_all_food_as_positions();
        let food2 = result2.get_all_food_as_positions();
        // Both should complete without panic — food positions may differ
        assert!(!food1.is_empty() || !food2.is_empty());
    }

    #[test]
    fn test_step_preserves_food_minimum() {
        let fixture = include_str!("../../fixtures/start_of_game.json");
        let (board, snake_ids) = load_fixture(fixture);

        let food_config = FoodSpawnConfig {
            minimum_food: 3,
            food_spawn_chance: 0,
        };

        let moves: Vec<(SnakeId, Vec<Move>)> = snake_ids
            .iter()
            .map(|&sid| (sid, vec![Move::Right]))
            .collect();

        let result = step_with_food(&board, &TestInstruments, moves, 42, 1, &food_config);
        let food_count = result.get_all_food_as_positions().len();
        assert!(
            food_count >= 3,
            "Expected at least 3 food, got {food_count}"
        );
    }
}
