import { describe, it, expect, vi, beforeEach } from "vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";

const mocks = vi.hoisted(() => ({
  searchLocal: vi.fn(),
  searchServer: vi.fn(),
}));

vi.mock("../../ipc/search", () => ({
  searchLocal: mocks.searchLocal,
  searchServer: mocks.searchServer,
}));

import { SearchBox } from "../../components/SearchBox";

function Wrapper({ onPick }: { onPick: (h: never) => void }) {
  return <SearchBoxHost onPick={onPick} addr="127.0.0.1:9999" />;
}

function SearchBoxHost({ onPick, addr }: { onPick: (h: never) => void; addr?: string }) {
  const [query, setQuery] = useState("");
  return <SearchBox query={query} onQueryChange={setQuery} onPick={onPick} addr={addr} />;
}

beforeEach(() => {
  vi.resetModules();
  mocks.searchLocal.mockReset();
  mocks.searchServer.mockReset();
});

describe("<SearchBox>", () => {
  it("renders input with placeholder", () => {
    render(<Wrapper onPick={() => {}} />);
    expect(screen.getByPlaceholderText(/search/i)).toBeInTheDocument();
  });

  it("does not render dropdown when query is empty", () => {
    mocks.searchLocal.mockResolvedValue([]);
    mocks.searchServer.mockResolvedValue([]);
    render(<Wrapper onPick={() => {}} />);
    expect(screen.queryByTestId("search-dropdown")).toBeNull();
  });

  it("shows hits grouped by kind when results arrive", async () => {
    mocks.searchLocal.mockResolvedValue([
      { kind: "Pipeline", id: "1", title: "alpha-pipe", path: ["Pipelines"] },
      { kind: "Application", id: "2", title: "alpha-app", path: ["Applications"] },
    ]);
    mocks.searchServer.mockResolvedValue([
      {
        kind: "ClusterNode",
        id: "127.0.0.1:9999",
        title: "127.0.0.1:9999",
        path: ["Cluster"],
      },
    ]);

    render(<Wrapper onPick={() => {}} />);
    const input = screen.getByPlaceholderText(/search/i);
    fireEvent.change(input, { target: { value: "alpha" } });

    expect(await screen.findByTestId("search-dropdown")).toBeInTheDocument();
    expect(await screen.findByText("alpha-pipe")).toBeInTheDocument();
    expect(screen.getByText("alpha-app")).toBeInTheDocument();
    expect(screen.getByText("127.0.0.1:9999")).toBeInTheDocument();
    expect(screen.getByText("Pipelines")).toBeInTheDocument();
    expect(screen.getByText("Applications")).toBeInTheDocument();
  });

  it("emits onPick when a hit is clicked", async () => {
    mocks.searchLocal.mockResolvedValue([
      { kind: "Pipeline", id: "42", title: "alpha-pipe", path: ["Pipelines"] },
    ]);
    mocks.searchServer.mockResolvedValue([]);

    const onPick = vi.fn();
    render(<Wrapper onPick={onPick} />);
    fireEvent.change(screen.getByPlaceholderText(/search/i), {
      target: { value: "alpha" },
    });
    fireEvent.click(await screen.findByText("alpha-pipe"));
    expect(onPick).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "Pipeline", id: "42" }),
    );
  });
});
