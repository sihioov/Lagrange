import {
  ResearchTerminalShell,
  type ResearchTerminalShellProps,
} from "@/components/shell/research-terminal-shell";

export type StockBetaTerminalShellProps = Omit<
  ResearchTerminalShellProps,
  "sessionLabel" | "shellKind"
> & {
  readonly readOnlyLabel: string;
};

export function StockBetaTerminalShell({ readOnlyLabel, ...props }: StockBetaTerminalShellProps) {
  return (
    <ResearchTerminalShell
      {...props}
      sessionLabel={readOnlyLabel}
      shellKind="stock-beta-terminal"
    />
  );
}
