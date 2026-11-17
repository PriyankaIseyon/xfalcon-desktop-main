import React from "react";
import { cn } from "../lib/cn";
import { Label } from "./Label";
import { controlBase, controlSizes } from "./controlStyles";

export type InputProps = React.InputHTMLAttributes<HTMLInputElement>;

export const Input = React.forwardRef<HTMLInputElement, InputProps>(({ className, ...props }, ref) => (
  <input ref={ref} className={cn(controlBase, controlSizes.md, className)} {...props} />
));
Input.displayName = "Input";

export type TextareaProps = React.TextareaHTMLAttributes<HTMLTextAreaElement>;

export const Textarea = React.forwardRef<HTMLTextAreaElement, TextareaProps>(({ className, ...props }, ref) => (
  <textarea ref={ref} className={cn(controlBase, "resize-none py-2", className)} {...props} />
));
Textarea.displayName = "Textarea";

export interface FieldProps {
  label?: React.ReactNode;
  hint?: React.ReactNode;
  error?: React.ReactNode;
  required?: boolean;
  children: React.ReactNode;
  className?: string;
  /** Explicit control id. Optional — the field mints one when it is omitted. */
  htmlFor?: string;
}

/** The props a field injects into its control. Named so the clone below is
 * checked against something rather than cast into place. */
type ControlProps = {
  id?: string;
  required?: boolean;
  "aria-invalid"?: boolean;
  "aria-labelledby"?: string;
  "aria-describedby"?: string;
};

/**
 * Label, control, hint, and error as one unit.
 *
 * # The id is minted here
 *
 * A label is only a label when it points at something. This used to pass
 * `htmlFor` straight through to `Label` and to the control, so a caller that
 * did not supply one got a `<label>` with no `for` and an `<input>` with no
 * `id` — a caption floating beside an unnamed control, unreachable by click
 * and unnamed to a screen reader. Sixteen of twenty call sites hand-wired an
 * id to work around that, and the two that did not shipped the defect.
 *
 * An explicit `htmlFor` still wins, and so does an id the control already
 * carries; the generated one is the floor, not an override.
 *
 * # Hint and error are not alternatives
 *
 * The hint used to be REPLACED by the error, so the sentence explaining the
 * expected format vanished at the exact moment a person got the format wrong.
 * Both render, and both are named in `aria-describedby`, in reading order.
 */
export function Field({ label, hint, error, required, children, className, htmlFor }: FieldProps) {
  const uid = React.useId();
  const errorId = `${uid}-error`;
  const hintId = `${uid}-hint`;
  const labelId = `${uid}-label`;

  const element = React.isValidElement<ControlProps>(children) ? children : null;
  const controlId = htmlFor ?? element?.props.id ?? `${uid}-control`;

  const child = element
    ? React.cloneElement(element, {
        id: controlId,
        required: required ?? element.props.required,
        // `for` names a labelable element. It does NOT name a button, and
        // `Select` is a button, so a select in a field announced its current
        // option and never the field. `aria-labelledby` names anything, and
        // for a native input it resolves to the same string `for` already
        // gave, so this is a fix for one control and a no-op for the rest.
        "aria-labelledby": label ? (element.props["aria-labelledby"] ?? labelId) : element.props["aria-labelledby"],
        "aria-invalid": error ? true : element.props["aria-invalid"],
        // Merged, not replaced: a control may already point at something of
        // its own, and dropping that description is how a field ends up
        // describing less after it gains a hint.
        "aria-describedby":
          [element.props["aria-describedby"], hint ? hintId : null, error ? errorId : null].filter(Boolean).join(" ") ||
          undefined,
      })
    : children;

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      {label ? (
        <Label htmlFor={controlId} id={labelId} required={required}>
          {label}
        </Label>
      ) : null}
      {child}
      {hint ? (
        <span className="type-caption text-muted" id={hintId}>
          {hint}
        </span>
      ) : null}
      {error ? (
        <span className="type-caption text-danger" id={errorId} role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}
