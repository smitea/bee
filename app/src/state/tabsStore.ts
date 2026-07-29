import { create } from "zustand";

import * as ipc from "../ipc/tabs";

export type TabKind =
  | "cluster"
  | "application"
  | "application_pipelines"
  | "application_datasources"
  | "pipeline"
  | "datasource";

export interface TabRow {
  id: number;
  kind: TabKind;
  resource_id: string | null;
  title: string;
  pinned: boolean;
  position: number;
}

export interface TabsStore {
  tabs: TabRow[];
  activeId: number | null;
  hydrated: boolean;
  hydrate(): Promise<void>;
  open(input: { kind: TabKind; resourceId: string | null; title: string }): Promise<void>;
  close(id: number): Promise<void>;
  setActive(id: number | null): Promise<void>;
  pin(id: number, pinned: boolean): Promise<void>;
}

function asKind(k: string): TabKind {
  switch (k) {
    case "cluster":
    case "application":
    case "application_pipelines":
    case "application_datasources":
    case "pipeline":
    case "datasource":
      return k;
    default:
      return "cluster";
  }
}

export const useTabs = create<TabsStore>((set, get) => ({
  tabs: [],
  activeId: null,
  hydrated: false,
  async hydrate() {
    const [list, state] = await Promise.all([ipc.tabsList(), ipc.workspaceState()]);
    const hasCluster = list.some((t) => t.kind === "cluster");
    let working = list;
    if (!hasCluster) {
      const id = await ipc.tabOpen("cluster", null, "Cluster");
      working = [
        ...list,
        {
          id,
          kind: "cluster",
          resourceId: null,
          title: "Cluster",
          pinned: false,
          position: 0,
        },
      ];
    }
    const initialActive = state.activeTabId ?? working[0]?.id ?? null;
    if (initialActive !== null) {
      await ipc.tabSetActive(initialActive);
    }
    set({
      tabs: working.map((t) => ({
        id: t.id,
        kind: asKind(t.kind),
        resource_id: t.resourceId,
        title: t.title,
        pinned: t.pinned,
        position: t.position,
      })),
      activeId: initialActive,
      hydrated: true,
    });
  },
  async open(input) {
    const id = await ipc.tabOpen(input.kind, input.resourceId, input.title);
    const list = await ipc.tabsList();
    await ipc.tabSetActive(id);
    set({
      tabs: list.map((t) => ({
        id: t.id,
        kind: asKind(t.kind),
        resource_id: t.resourceId,
        title: t.title,
        pinned: t.pinned,
        position: t.position,
      })),
      activeId: id,
    });
  },
  async close(id) {
    await ipc.tabClose(id);
    const list = await ipc.tabsList();
    const current = get().activeId;
    const nextActive = current === id ? (list[0]?.id ?? null) : current;
    if (nextActive !== null) {
      await ipc.tabSetActive(nextActive);
    }
    set({
      tabs: list.map((t) => ({
        id: t.id,
        kind: asKind(t.kind),
        resource_id: t.resourceId,
        title: t.title,
        pinned: t.pinned,
        position: t.position,
      })),
      activeId: nextActive,
    });
  },
  async setActive(id) {
    if (id !== null) {
      await ipc.tabSetActive(id);
    }
    set({ activeId: id });
  },
  async pin(id, pinned) {
    await ipc.tabPin(id, pinned);
    const list = await ipc.tabsList();
    set({
      tabs: list.map((t) => ({
        id: t.id,
        kind: asKind(t.kind),
        resource_id: t.resourceId,
        title: t.title,
        pinned: t.pinned,
        position: t.position,
      })),
    });
  },
}));