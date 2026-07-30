import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ClusterStatusDot } from "../../components/ClusterStatusDot";

describe("<ClusterStatusDot>", () => {
  it("applies the green data-status when the profile matches the active connected cluster", () => {
    render(
      <ClusterStatusDot
        profileAddr="127.0.0.1:9999"
        activeAddr="127.0.0.1:9999"
        status={{ kind: "Connected" }}
      />,
    );
    expect(screen.getByRole("status").getAttribute("data-status")).toBe("green");
  });

  it("applies amber when the profile differs from the active address", () => {
    render(
      <ClusterStatusDot
        profileAddr="10.0.0.1:9999"
        activeAddr="127.0.0.1:9999"
        status={{ kind: "Connected" }}
      />,
    );
    expect(screen.getByRole("status").getAttribute("data-status")).toBe("amber");
  });

  it("applies red when the profile matches but the connection errored", () => {
    render(
      <ClusterStatusDot
        profileAddr="127.0.0.1:9999"
        activeAddr="127.0.0.1:9999"
        status={{ kind: "Error", reason: "refused" }}
      />,
    );
    expect(screen.getByRole("status").getAttribute("data-status")).toBe("red");
  });
});