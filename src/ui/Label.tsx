import React from "react";
import { cn } from "../lib/cn";

export interface LabelProps extends React.LabelHTMLAttributes<HTMLLabelElement> {
  /** Renders as a required-field marker for assistive tech. */
  required?: boolean;
}

export function Label({ required, className, children, ...props }: LabelProps) {
  return (
    <label className={cn("type-label block font-medium text-secondary", className)} {...props}>
      {children}
      {required ? (
        <span aria-hidden className="text-danger">
          {" "}
          *
        </span>
      ) : null}
      {/* The separator is inside the span and is a comma, not a space. The
          accessible-name algorithm trims each node's contribution, so a leading
          space is dropped and the note runs into the label -- "Header
          row(required)". The same trim already bit `Link`'s destination note. */}
      {required ? <span className="sr-only">, required</span> : null}
    </label>
  );
}
