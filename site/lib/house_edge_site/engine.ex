defmodule HouseEdgeSite.Engine do
  use Rustler, otp_app: :house_edge_sit, crate: "houseedge_nif"

  def start_run(_config), do: :erlang.nif_error(:nif_not_loaded)
  def advance(_handle, _rounds), do: :erlang.nif_error(:nif_not_loaded)
  def paths(_handle), do: :erlang.nif_error(:nif_not_loaded)
  def summary(_handle), do: :erlang.nif_error(:nif_not_loaded)

  def trial(paths, trial_index, checkpoints) do
    offset = trial_index * checkpoints * 8
    <<_::binary-size(offset), row::binary-size(checkpoints * 8), _::binary>> = paths
    for <<v::float-little-64 <- row>>, do: v
  end
end
