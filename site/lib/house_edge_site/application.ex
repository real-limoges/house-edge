defmodule HouseEdgeSite.Application do
  @moduledoc false

  use Application

  @impl true
  def start(_type, _args) do
    children = [
      HouseEdgeSiteWeb.Telemetry,
      {DNSCluster, query: Application.get_env(:house_edge_site, :dns_cluster_query) || :ignore},
      {Phoenix.PubSub, name: HouseEdgeSite.PubSub},
      # Before the endpoint, so no request finds the figures missing.
      HouseEdgeSite.Figures.Loader,
      # Engine work runs only under this supervisor.
      {Task.Supervisor, name: HouseEdgeSite.TaskSupervisor},
      HouseEdgeSiteWeb.Endpoint
    ]

    Supervisor.start_link(children, strategy: :one_for_one, name: HouseEdgeSite.Supervisor)
  end

  # Tell Phoenix to update the endpoint configuration on a hot upgrade.
  @impl true
  def config_change(changed, _new, removed) do
    HouseEdgeSiteWeb.Endpoint.config_change(changed, removed)
    :ok
  end
end
