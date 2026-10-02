defmodule HouseEdgeSite.Figures do
  def load! do
    for chapter <- [:roulette, :baccarat, :blackjack, :craps, :parlays] do
      :persistent_term.put({__MODULE__, chapter}, compute(chapter))
    end

    :ok
  end

  def get(chapter), do: :persistent_term.get({__MODULE__, chapter})
end
