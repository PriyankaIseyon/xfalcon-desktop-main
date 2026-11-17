import React from "react";
import { Upload } from "lucide-react";
import { cn } from "../lib/cn";

export interface FileFieldProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "type"> {
  label?: string;
  hint?: string;
  error?: string;
  onFiles?: (files: FileList) => void;
}

export const FileField = React.forwardRef<HTMLInputElement, FileFieldProps>(
  ({ className, label = "Choose file", hint, error, id, disabled, onChange, onFiles, ...props }, ref) => {
    const autoId = React.useId();
    const inputId = id ?? autoId;
    const [dragOver, setDragOver] = React.useState(false);

    return (
      <div className={cn("flex flex-col gap-1.5", className)}>
        <div
          className={cn(
            "surface-card flex flex-col items-center gap-2 border-dashed px-4 py-6 text-center transition-colors duration-fast",
            dragOver && "border-accent bg-accent-muted",
            error && "border-danger",
            disabled && "opacity-50",
          )}
          onDragEnter={(event) => {
            event.preventDefault();
            if (!disabled) setDragOver(true);
          }}
          onDragLeave={() => setDragOver(false)}
          onDragOver={(event) => event.preventDefault()}
          onDrop={(event) => {
            event.preventDefault();
            setDragOver(false);
            if (disabled) return;
            const files = event.dataTransfer.files;
            if (files.length === 0) return;
            onFiles?.(files);
          }}
        >
          <Upload aria-hidden className="icon-glyph-prominent text-muted" />
          <p className="type-body text-secondary">{hint ?? "Drop a file here or browse"}</p>
          {/* The input comes FIRST because the label styles itself from it:
              `peer-*` compiles to a sibling combinator, which only looks
              forward. The input is the focusable control and is visually
              hidden, so without this Tab lands here and draws the ring on a
              1px box nobody can see. Same construction `Radio` uses, and the
              same ring `index.css` declares. */}
          <input
            ref={ref}
            id={inputId}
            type="file"
            className="peer sr-only"
            disabled={disabled}
            onChange={onChange}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? `${inputId}-error` : undefined}
            {...props}
          />
          <label
            htmlFor={inputId}
            className={cn(
              "press touch-target inline-flex h-control-compact cursor-pointer items-center rounded-md border border-default bg-card px-3 text-xs font-medium text-foreground",
              "hover:bg-hover",
              "peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-[color:var(--xf-focus)]",
              // Every other disabled control in the app dims to 50% opacity;
              // `pointer-events-none` alone left this the one exception that
              // still looked clickable while doing nothing.
              disabled && "pointer-events-none opacity-50",
            )}
          >
            {label}
          </label>
        </div>
        {error ? (
          <p className="type-caption text-danger" id={`${inputId}-error`} role="alert">
            {error}
          </p>
        ) : null}
      </div>
    );
  },
);
FileField.displayName = "FileField";
