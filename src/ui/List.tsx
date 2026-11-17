import React from "react";
import { cn } from "../lib/cn";

export function List({ className, ...props }: React.HTMLAttributes<HTMLUListElement>) {
  return <ul className={cn("m-0 list-none p-0", className)} {...props} />;
}

/**
 * A row in a list.
 *
 * There is deliberately no `interactive` flag. It painted `.nav-item` on a bare
 * `<li>`, which made a row LOOK like a control while staying unfocusable,
 * unreachable by keyboard, and announced as plain text -- and no caller ever
 * used it. A row that acts puts a real control inside itself.
 */
export type ListItemProps = React.LiHTMLAttributes<HTMLLIElement>;

export function ListItem({ className, ...props }: ListItemProps) {
  return <li className={className} {...props} />;
}
