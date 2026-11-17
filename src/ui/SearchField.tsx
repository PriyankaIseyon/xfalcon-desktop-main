import React from "react";
import { Search, X } from "lucide-react";
import { cn } from "../lib/cn";
import { controlBase, controlSizes } from "./controlStyles";
import { IconButton } from "./IconButton";

export interface SearchFieldProps extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "type"> {
  onClear?: () => void;
}

export const SearchField = React.forwardRef<HTMLInputElement, SearchFieldProps>(
  ({ className, value, defaultValue, onClear, ...props }, ref) => {
    const hasValue = Boolean(value ?? defaultValue);

    return (
      <div className={cn("relative flex items-center", className)}>
        <Search aria-hidden className="icon-glyph-default pointer-events-none absolute left-3 text-muted" />
        <input
          ref={ref}
          type="search"
          value={value}
          defaultValue={defaultValue}
          // `search-cancel` suppresses the engine's own clear button. WebView2
          // is Chromium and draws one inside `type="search"`, which put a
          // second, differently-shaped X beside the app's Clear control and
          // cleared the field without telling the app.
          className={cn(controlBase, controlSizes.md, "search-cancel pl-9 pr-9")}
          {...props}
        />
        {hasValue && onClear ? (
          <IconButton
            aria-label="Clear search"
            className="absolute right-1 touch-target"
            size="sm"
            type="button"
            onClick={onClear}
          >
            <X aria-hidden />
          </IconButton>
        ) : null}
      </div>
    );
  },
);
SearchField.displayName = "SearchField";
