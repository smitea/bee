import { useEffect, useMemo, useState } from "react";
import { Search } from "lucide-react";

import type { SearchHit } from "../ipc/search";
import { searchLocal, searchServer } from "../ipc/search";

const DEBOUNCE_MS = 200;

interface Props {
  query: string;
  onQueryChange(q: string): void;
  onPick(hit: SearchHit): void;
  addr?: string;
}

export function SearchBox({ query, onQueryChange, onPick, addr }: Props) {
  const [hits, setHits] = useState<SearchHit[]>([]);
  const open = query.length > 0;

  useEffect(() => {
    if (!query) {
      setHits([]);
      return;
    }
    const handle = setTimeout(async () => {
      const [local, server] = await Promise.all([
        searchLocal(query).catch(() => [] as SearchHit[]),
        addr ? searchServer(query).catch(() => [] as SearchHit[]) : Promise.resolve([] as SearchHit[]),
      ]);
      setHits([...local, ...server]);
    }, DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [query, addr]);

  const grouped = useMemo(() => {
    const map = new Map<string, SearchHit[]>();
    for (const h of hits) {
      const arr = map.get(h.kind) ?? [];
      arr.push(h);
      map.set(h.kind, arr);
    }
    return Array.from(map.entries());
  }, [hits]);

  return (
    <div className="relative">
      <div className="relative">
        <Search
          size={11}
          className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-400"
        />
        <input
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          placeholder="Search applications, pipelines, dashboards"
          className="w-full pl-7 pr-2 py-1 text-xs bg-gray-100 dark:bg-neutral-700 rounded border-0 focus:outline-none focus:ring-1 focus:ring-accent-blue"
        />
      </div>
      {open && grouped.length > 0 && (
        <div
          data-testid="search-dropdown"
          className="absolute z-30 left-0 right-0 mt-1 max-h-80 overflow-auto rounded-md border border-gray-200 dark:border-neutral-700 bg-white dark:bg-neutral-800 shadow-lg text-xs"
        >
          {grouped.map(([kind, list]) => (
            <div key={kind}>
              <div className="px-2 py-1 text-[10px] uppercase tracking-wider text-gray-500 dark:text-neutral-400 bg-gray-50 dark:bg-neutral-700/40">
                {kind} ({list.length})
              </div>
              <ul>
                {list.map((hit) => {
                  const path = hit.path.join(" / ");
                  return (
                    <li key={`${hit.kind}:${hit.id}`}>
                      <button
                        type="button"
                        onClick={() => {
                          onQueryChange("");
                          setHits([]);
                          onPick(hit);
                        }}
                        className="w-full text-left px-2 py-1.5 hover:bg-gray-50 dark:hover:bg-neutral-700/50 flex flex-col"
                      >
                        <span className="font-mono">{hit.title}</span>
                        <span className="text-[10px] text-gray-400">{path}</span>
                      </button>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
