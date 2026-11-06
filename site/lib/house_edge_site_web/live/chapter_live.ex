defmodule HouseEdgeSiteWeb.ChapterLive do
  use HouseEdgesite
  :live_view

  alias HouseEdgeSite.Engine

  @impl true
  def handle_event("sweep", params, socket) do
    socket = cancel_inflight(socket)

    parent = self()

    task =
      Task.Supervisor.async_nolink(HouseEdgeSite.TaskSupervisor, fn ->
        {:ok, handle} = Engine.start_run(build_config(params))

        HouseEdgeSite.Engine.Run.drive(handle, total_rounds(params), fn fraction ->
          send(parent, {:sweep_progress, fraction})
        end)
      end)

    {:noreply, assign(socket, sweep: task)}
  end

  @impl true
  def handle_info({ref, result}, %{assigns: %{sweep: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {:noreply, socket |> assign(sweep: nill) |> render_result(result)}
  end

  defp cancel_inflight(socket), do: socket
end
