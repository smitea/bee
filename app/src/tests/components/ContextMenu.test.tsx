import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { ContextMenu, type ContextMenuItem } from "../../components/ContextMenu";

const items: ContextMenuItem[] = [
  { id: "close", label: "Close", onSelect: vi.fn() },
  { id: "others", label: "Close Others", onSelect: vi.fn() },
  { id: "right", label: "Close to the Right", onSelect: vi.fn(), disabled: true },
  { id: "pin", label: "Pin", onSelect: vi.fn() },
];

describe("<ContextMenu>", () => {
  it("renders nothing when closed", () => {
    const { container } = render(<ContextMenu open={false} items={items} onClose={() => {}} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders the items when open", () => {
    render(<ContextMenu open items={items} onClose={() => {}} />);
    expect(screen.getByText("Close")).toBeInTheDocument();
    expect(screen.getByText("Close Others")).toBeInTheDocument();
    expect(screen.getByText("Close to the Right")).toBeInTheDocument();
    expect(screen.getByText("Pin")).toBeInTheDocument();
  });

  it("invokes the matching onSelect and closes when a non-disabled item is clicked", () => {
    const onClose = vi.fn();
    const onSelect = vi.fn();
    const local: ContextMenuItem[] = [
      { id: "close", label: "Close", onSelect },
    ];
    render(<ContextMenu open items={local} onClose={onClose} />);
    fireEvent.click(screen.getByText("Close"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does not invoke onSelect for disabled items", () => {
    const onSelect = vi.fn();
    const local: ContextMenuItem[] = [
      { id: "x", label: "Disabled", onSelect, disabled: true },
    ];
    render(<ContextMenu open items={local} onClose={() => {}} />);
    fireEvent.click(screen.getByText("Disabled"));
    expect(onSelect).not.toHaveBeenCalled();
  });
});