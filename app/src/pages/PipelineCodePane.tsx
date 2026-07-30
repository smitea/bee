import CodeMirror from "@uiw/react-codemirror";
import { sql } from "@codemirror/lang-sql";
import { oneDark } from "@codemirror/theme-one-dark";

export type CodePaneMode = "dag" | "sql";

interface Props {
  value: string;
  onChange(value: string): void;
  theme: "light" | "dark";
}

export function PipelineCodePane({ value, onChange, theme }: Props) {
  return (
    <CodeMirror
      value={value}
      height="380px"
      theme={theme === "dark" ? oneDark : "light"}
      extensions={[sql()]}
      onChange={onChange}
    />
  );
}