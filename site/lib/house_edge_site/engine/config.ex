defmodule HouseEdgeSite.Engine.Config do
  @moduledoc """
  `HouseEdgeSite.Engine.star_run/1`` decodes + validates this`.

  Validation happens here. Invalid input reaches an assert inside the engine
  constructor. Asserts reaching the NIF boundary is a panic.

  Erlang floads cannot be NaN or infinite, so checks are sign + range.
  """

  @keys ~w(game variant policy legs rho trials rounds bankroll base table_max
   checkpoints seed rounds_per_hour)a

  @games ~w(roulette_straight_up roulette_even_money baccarat_banker baccarat_player
   craps_any_seven craps_pass_line blackjack parlay)a

  @policy ~w(flat martingale fibonacci dalbert)a

  @doc "Reject anything that would trip an assert on the other side."
  def validate(cfg) when is_map(cfg) do
    case @keys -- Map.keys(cfg) do
      [] -> validate_values(cfg)
      missing -> {:error, {:missing_keys, missing}}
    end
  end

  defp validate_values(%{game: game })
end
