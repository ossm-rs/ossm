import { Badge } from "@radix-ui/themes";
import { indexPartsFor } from "../model/catalog";
import type { PrintFile } from "../model/catalog";

const isCad = (f: PrintFile) => f.format === "step" || f.format === "stp";

function FileItem({ file }: { file: PrintFile }) {
  return (
    <li>
      <a href={file.url} target="_blank" rel="noreferrer noopener">
        {file.name}
      </a>{" "}
      {file.category && (
        <Badge
          size="1"
          variant="soft"
          color={file.category === "strength" ? "orange" : "gray"}
        >
          {file.category}
        </Badge>
      )}{" "}
      {!file.exact && (
        <Badge
          size="1"
          variant="outline"
          color="gray"
          title="Not identical to the modelled part: another revision or print option"
        >
          ≈
        </Badge>
      )}
    </li>
  );
}

/** Upstream print/CAD files for some model nodes, from the generated parts index. */
export default function PartFiles({
  instances,
  className,
}: {
  instances: string[];
  className?: string;
}) {
  const files = [
    ...new Map(
      indexPartsFor(instances)
        .flatMap((p) => p.files)
        .map((f) => [f.url, f]),
    ).values(),
  ];
  const sections: Array<[string, PrintFile[]]> = [
    ["Print files", files.filter((f) => !isCad(f))],
    ["CAD", files.filter(isCad)],
  ];
  return (
    <div className={className}>
      {sections
        .filter(([, list]) => list.length > 0)
        .map(([title, list]) => (
          <div key={title}>
            <p>
              <strong>{title}</strong>
            </p>
            <ul>
              {list.map((f) => (
                <FileItem key={f.url} file={f} />
              ))}
            </ul>
          </div>
        ))}
    </div>
  );
}

