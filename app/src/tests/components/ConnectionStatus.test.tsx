import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { ConnectionStatusView } from "../../components/ConnectionStatus";

describe("<ConnectionStatusView>", () => {
  it("renders a solid green dot and Connected label", () => {
    render(
      <ConnectionStatusView
        addr="1.2.3.4:5"
        status={{ kind: "Connected" }}
        onOpenSettings={vi.fn()}
      />,
    );
    expect(screen.getByText("1.2.3.4:5")).toBeInTheDocument();
    expect(screen.getByText("Connected")).toBeInTheDocument();
  });

  it("renders a pulsing green dot when Connecting", () => {
    const { container } = render(
      <ConnectionStatusView
        addr="x"
        status={{ kind: "Connecting" }}
        onOpenSettings={vi.fn()}
      />,
    );
    const dot = container.querySelector("span");
    expect(dot?.className).toMatch(/animate-pulse/);
  });

  it("renders a red dot and a settings link when in Error state", () => {
    const onOpen = vi.fn();
    render(
      <ConnectionStatusView
        addr="x"
        status={{ kind: "Error", reason: "refused" }}
        onOpenSettings={onOpen}
      />,
    );
    fireEvent.click(screen.getByText("Open connection settings"));
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it("renders a red dot when Disconnected", () => {
    const { container } = render(
      <ConnectionStatusView
        addr="x"
        status={{ kind: "Disconnected" }}
        onOpenSettings={vi.fn()}
      />,
    );
    const dot = container.querySelector("span");
    expect(dot?.className).toMatch(/bg-accent-red/);
  });
});
