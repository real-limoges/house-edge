defmodule HouseEdgeSite.Engine.Config do
  @moduledoc """
  """

  @doc "Reject anything that would trip an assert on the other side."
  def validate(%{outcomes: outcomes}) when is_list(outcomes) do
    total = Enum.sum(Enum.map(outcomes, fn {p, _net} -> p end))

    cond do
      outcomes == [] -> {:error, :empty_table}
      Enum.any?(outcomes, fn {p, _} -> p < 0 end) -> {:error, :negative_probability}
      abs(total - 1.0) > 1.0e-9 -> {:error, {:probabilities_sum_to, total}}
      true -> :ok
    end
  end
end
