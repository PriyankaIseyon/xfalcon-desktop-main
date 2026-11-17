import React from "react";
import { cn } from "../lib/cn";
import { controlBase, controlSizes } from "./controlStyles";

export type DateFieldProps = React.InputHTMLAttributes<HTMLInputElement>;

export const DateField = React.forwardRef<HTMLInputElement, DateFieldProps>(({ className, ...props }, ref) => (
  <input ref={ref} type="date" className={cn(controlBase, controlSizes.md, className)} {...props} />
));
DateField.displayName = "DateField";

export type TimeFieldProps = React.InputHTMLAttributes<HTMLInputElement>;

export const TimeField = React.forwardRef<HTMLInputElement, TimeFieldProps>(({ className, ...props }, ref) => (
  <input ref={ref} type="time" className={cn(controlBase, controlSizes.md, className)} {...props} />
));
TimeField.displayName = "TimeField";

export type DateTimeFieldProps = React.InputHTMLAttributes<HTMLInputElement>;

export const DateTimeField = React.forwardRef<HTMLInputElement, DateTimeFieldProps>(({ className, ...props }, ref) => (
  <input ref={ref} type="datetime-local" className={cn(controlBase, controlSizes.md, className)} {...props} />
));
DateTimeField.displayName = "DateTimeField";
