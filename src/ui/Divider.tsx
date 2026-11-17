import { cn } from "../lib/cn";

export interface DividerProps extends React.HTMLAttributes<HTMLHRElement> {
  /** Vertical rule for toolbars; default is horizontal. */
  orientation?: "horizontal" | "vertical";
}

export function Divider({ orientation = "horizontal", className, ...props }: DividerProps) {
  return (
    <hr
      role="separator"
      aria-orientation={orientation}
      className={cn(
        "shrink-0 border-0 bg-hairline",
        orientation === "horizontal" ? "h-px w-full" : "h-6 w-px",
        className,
      )}
      {...props}
    />
  );
}
