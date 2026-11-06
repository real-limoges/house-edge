defmodule HouseEdgeSiteWeb.PageController do
  use HouseEdgeSiteWeb, :controller

  def home(conn, _params) do
    render(conn, :home)
  end
end
