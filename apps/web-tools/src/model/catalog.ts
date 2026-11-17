import data from "./ossm-alt.parts.json";

// Generated alongside the model by `just cad export-web`: which index part each
// model node is, and per part its kind, count and upstream print files.

export type PrintFile = {
  name: string;
  url: string;
  format: string;
  /** False when the file is a different revision/option of the modelled part. */
  exact: boolean;
  category?: string;
  quantity?: number;
};

export type IndexPart = {
  name: string;
  kind: string;
  count: number;
  files: PrintFile[];
};

const instances: Record<string, string> = data.instances;
const parts: Record<string, IndexPart> = data.parts;

export const MODEL_SOURCE = data.source;

/** The index part behind one model node, if any. */
export function indexPartOf(instance: string): IndexPart | undefined {
  const id = instances[instance];
  return id ? parts[id] : undefined;
}

/** The distinct index parts behind a set of model nodes (extras have none). */
export function indexPartsFor(instanceIds: string[]): IndexPart[] {
  const ids = new Set(
    instanceIds.map((i) => instances[i]).filter((p): p is string => !!p),
  );
  return [...ids].map((id) => parts[id]);
}
