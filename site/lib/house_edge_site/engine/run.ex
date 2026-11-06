defmodule HouseEdgeSite.Engine.Run do
  @rounds_per_chunk 200_000

  def drive(handle, total_rounds, on_progress) do
    do_drive(handle, total_rounds, 0, on_progress)
  end

  defp do_drive(handle, total, done, _on_progress) when done >= total do
    {:ok, Engine.summary(handle)}
  end

  defp do_drive(handle, total, done, on_progress) do
    chunk = min(@rounds_per_chunk, total - done)
    :ok = Engine.advance(handle, chunk)
    done = done + chunk

    case on_progress.(done / total) do
      :halt -> {:halted, done}
      _ -> do_drive(handle, total, done, on_progress)
    end
  end
end
