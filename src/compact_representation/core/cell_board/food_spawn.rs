//! Deterministic food spawning using MINSTD PRNG.
//!
//! Implements the same food spawning logic as the official Battlesnake rules:
//! 1. Check if food count is below minimum → spawn to reach minimum
//! 2. Roll food spawn chance → spawn 1 if roll succeeds
//! 3. Collect unoccupied points (excluding food, snake bodies, and cells
//!    adjacent to snake heads)
//! 4. Shuffle unoccupied points using Fisher-Yates
//! 5. Place food at the first N positions


use super::CellBoard;
use crate::compact_representation::core::dimensions::Dimensions;
use crate::compact_representation::core::{CellIndex, CellNum};
use crate::minstd::MinstdRand;
use crate::types::{FoodGettableGame, Move};
use crate::wire_representation::Position;

/// Configuration for food spawning behavior.
#[derive(Debug, Copy, Clone)]
pub struct FoodSpawnConfig {
    /// Minimum food that must be on the board at all times.
    /// Default in standard games: 1
    pub minimum_food: usize,
    /// Percentage chance (0-100) of spawning a new food each turn.
    /// Default in standard games: 15
    pub food_spawn_chance: u32,
}

impl Default for FoodSpawnConfig {
    fn default() -> Self {
        Self {
            minimum_food: 1,
            food_spawn_chance: 15,
        }
    }
}

impl<T: CellNum, D: Dimensions, const BOARD_SIZE: usize, const MAX_SNAKES: usize>
    CellBoard<T, D, BOARD_SIZE, MAX_SNAKES>
{
    /// Spawn food deterministically using the provided PRNG.
    ///
    /// This implements the official Battlesnake food spawning rules:
    /// - If current food count < `minimum_food`, spawn enough to reach the minimum
    /// - Otherwise, roll `food_spawn_chance` percent chance to spawn 1 food
    /// - Food is placed on randomly-selected unoccupied cells (not food, not snake,
    ///   not adjacent to any living snake's head)
    ///
    /// The `rng` should be a MINSTD PRNG seeded for the current turn via
    /// `minstd::rng_for_turn(game_seed, turn)`.
    ///
    /// **RNG call order** (must match Go rules for cross-engine verification):
    /// 1. `rng.intn(100)` — food spawn chance check
    /// 2. If food is needed: `rng.shuffle(unoccupied_points)` — Fisher-Yates
    pub fn spawn_food(&mut self, rng: &mut MinstdRand, config: &FoodSpawnConfig) {
        let food_needed = self.check_food_needed(rng, config);
        if food_needed > 0 {
            self.place_food_randomly(rng, food_needed);
        }
    }

    fn check_food_needed(&self, rng: &mut MinstdRand, config: &FoodSpawnConfig) -> usize {
        let current_food = self.get_all_food_as_native_positions().len();

        if current_food < config.minimum_food {
            return config.minimum_food - current_food;
        }

        if config.food_spawn_chance > 0
            && (100 - rng.intn(100) as u32) < config.food_spawn_chance
        {
            return 1;
        }

        0
    }

    fn place_food_randomly(&mut self, rng: &mut MinstdRand, n: usize) {
        let mut unoccupied = self.get_unoccupied_points_for_food();
        let n = n.min(unoccupied.len());

        rng.shuffle(&mut unoccupied);

        for &cell_idx in unoccupied.iter().take(n) {
            self.cells[cell_idx.0.as_usize()].set_food();
        }
    }

    /// Get unoccupied points suitable for food placement.
    ///
    /// Excludes: food cells, snake body/head cells, and cells adjacent to
    /// any living snake's head. Iterates in column-major order (x: 0..width,
    /// y: 0..height) to match the Go rules' `GetUnoccupiedPoints`.
    fn get_unoccupied_points_for_food(&self) -> Vec<CellIndex<T>> {
        let width = Self::width();
        let height = width; // boards are square

        // Build a set of cells adjacent to snake heads
        let mut head_adjacent = [false; BOARD_SIZE];
        for snake_idx in 0..MAX_SNAKES {
            if self.healths[snake_idx] == 0 {
                continue;
            }
            let head = self.heads[snake_idx];
            let head_pos = head.into_position(width);

            for m in &[Move::Up, Move::Down, Move::Left, Move::Right] {
                let adj = head_pos.add_vec(m.to_vector());
                if adj.x >= 0 && adj.x < width as i32 && adj.y >= 0 && adj.y < height as i32 {
                    let idx = CellIndex::<T>::new(adj, width);
                    head_adjacent[idx.0.as_usize()] = true;
                }
            }
        }

        // Collect unoccupied points in column-major order (x then y)
        // to match Go's GetUnoccupiedPoints iteration order
        let mut result = Vec::new();
        for x in 0..width as i32 {
            for y in 0..height as i32 {
                let idx = CellIndex::<T>::new(Position { x, y }, width);
                let cell = self.get_cell(idx);
                if cell.is_empty() && !head_adjacent[idx.0.as_usize()] {
                    result.push(idx);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact_representation::dimensions::Square;

    type TestBoard = CellBoard<u8, Square, { 11 * 11 }, 4>;

    fn load_fixture(json: &str) -> TestBoard {
        let game: crate::wire_representation::Game = serde_json::from_str(json).unwrap();
        let id_map = crate::types::build_snake_id_map(&game);
        TestBoard::convert_from_game(game, &id_map).unwrap()
    }

    #[test]
    fn test_spawn_food_minimum() {
        // Start of game fixture — should have food already
        let fixture = include_str!("../../../../fixtures/start_of_game.json");
        let mut board = load_fixture(fixture);

        // Remove all food manually, then spawn with min=1
        for i in 0..121 {
            let idx = CellIndex::<u8>::from_usize(i);
            if board.get_cell(idx).is_food() {
                board.cells[i].remove();
            }
        }
        assert_eq!(board.get_all_food_as_native_positions().len(), 0);

        let mut rng = MinstdRand::new(42);
        let config = FoodSpawnConfig {
            minimum_food: 1,
            food_spawn_chance: 0,
        };
        board.spawn_food(&mut rng, &config);
        assert_eq!(board.get_all_food_as_native_positions().len(), 1);
    }

    #[test]
    fn test_spawn_food_deterministic() {
        let fixture = include_str!("../../../../fixtures/start_of_game.json");
        let mut board1 = load_fixture(fixture);
        let mut board2 = load_fixture(fixture);

        // Remove all food from both
        for i in 0..121 {
            let idx = CellIndex::<u8>::from_usize(i);
            if board1.get_cell(idx).is_food() {
                board1.cells[i].remove();
            }
            if board2.get_cell(idx).is_food() {
                board2.cells[i].remove();
            }
        }

        let config = FoodSpawnConfig {
            minimum_food: 3,
            food_spawn_chance: 0,
        };

        let mut rng1 = MinstdRand::new(12345);
        let mut rng2 = MinstdRand::new(12345);
        board1.spawn_food(&mut rng1, &config);
        board2.spawn_food(&mut rng2, &config);

        let food1 = board1.get_all_food_as_positions();
        let food2 = board2.get_all_food_as_positions();
        assert_eq!(food1, food2);
        assert_eq!(food1.len(), 3);
    }

    #[test]
    fn test_spawn_food_chance_roll() {
        let fixture = include_str!("../../../../fixtures/start_of_game.json");

        let config = FoodSpawnConfig {
            minimum_food: 0,
            food_spawn_chance: 100, // always spawn
        };

        let mut board = load_fixture(fixture);
        let initial_food = board.get_all_food_as_native_positions().len();

        let mut rng = MinstdRand::new(42);
        board.spawn_food(&mut rng, &config);

        let new_food = board.get_all_food_as_native_positions().len();
        // With 100% spawn chance and min=0, should get exactly 1 more food
        assert_eq!(new_food, initial_food + 1);
    }

    #[test]
    fn test_no_food_on_heads_or_adjacent() {
        let fixture = include_str!("../../../../fixtures/start_of_game.json");
        let mut board = load_fixture(fixture);

        // Remove all food
        for i in 0..121 {
            if board.get_cell(CellIndex::<u8>::from_usize(i)).is_food() {
                board.cells[i].remove();
            }
        }

        let config = FoodSpawnConfig {
            minimum_food: 50, // Force lots of food to be placed
            food_spawn_chance: 0,
        };

        let mut rng = MinstdRand::new(42);
        board.spawn_food(&mut rng, &config);

        // Verify no food was placed on or adjacent to snake heads
        for snake_idx in 0..4 {
            if board.healths[snake_idx] == 0 {
                continue;
            }
            let head = board.heads[snake_idx];
            let head_pos = head.into_position(TestBoard::width());

            for m in &[Move::Up, Move::Down, Move::Left, Move::Right] {
                let adj = head_pos.add_vec(m.to_vector());
                if adj.x >= 0 && adj.x < 11 && adj.y >= 0 && adj.y < 11 {
                    let idx = CellIndex::<u8>::new(adj, 11);
                    assert!(
                        !board.get_cell(idx).is_food(),
                        "Food was placed adjacent to a snake head at ({}, {})",
                        adj.x,
                        adj.y
                    );
                }
            }
        }
    }
}
