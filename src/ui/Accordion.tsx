import React from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "../lib/cn";

export interface AccordionItemProps {
  id: string;
  title: React.ReactNode;
  children: React.ReactNode;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export interface AccordionProps {
  items: AccordionItemProps[];
  /** Allow multiple sections open at once. */
  multiple?: boolean;
  className?: string;
}

export function Accordion({ items, multiple = false, className }: AccordionProps) {
  const instanceId = React.useId();
  const triggerRefs = React.useRef<Array<HTMLButtonElement | null>>([]);
  const [openIds, setOpenIds] = React.useState<Set<string>>(() => {
    const initial = new Set<string>();
    for (const item of items) {
      if (item.open ?? item.defaultOpen) initial.add(item.id);
    }
    return initial;
  });

  const toggle = (id: string, next: boolean) => {
    setOpenIds((current) => {
      const updated = new Set(multiple ? current : []);
      if (next) updated.add(id);
      else updated.delete(id);
      return updated;
    });
    if (next && !multiple) {
      for (const openItem of items) {
        if (openItem.id !== id && (openItem.open ?? openIds.has(openItem.id))) {
          openItem.onOpenChange?.(false);
        }
      }
    }
    const item = items.find((entry) => entry.id === id);
    item?.onOpenChange?.(next);
  };

  const moveTriggerFocus = (event: React.KeyboardEvent, index: number) => {
    let nextIndex: number | undefined;
    if (event.key === "ArrowDown") nextIndex = (index + 1) % items.length;
    else if (event.key === "ArrowUp") nextIndex = (index - 1 + items.length) % items.length;
    else if (event.key === "Home") nextIndex = 0;
    else if (event.key === "End") nextIndex = items.length - 1;
    if (nextIndex == null) return;
    event.preventDefault();
    triggerRefs.current[nextIndex]?.focus();
  };

  return (
    <div className={cn("divide-y divide-hairline border-y border-hairline", className)}>
      {items.map((item, index) => {
        const isOpen = item.open ?? openIds.has(item.id);
        const panelId = `${instanceId}-panel-${index}`;
        const triggerId = `${instanceId}-trigger-${index}`;
        return (
          <section key={item.id}>
            <h3 className="m-0">
              <button
                ref={(node) => {
                  triggerRefs.current[index] = node;
                }}
                id={triggerId}
                aria-controls={panelId}
                aria-expanded={isOpen}
                className="touch-target press flex min-h-control-default w-full items-center gap-2 bg-transparent px-1 py-3 text-left text-xs font-medium text-secondary transition-colors duration-fast ease-soft hover:bg-hover hover:text-foreground"
                type="button"
                onClick={() => toggle(item.id, !isOpen)}
                onKeyDown={(event) => moveTriggerFocus(event, index)}
              >
                <ChevronRight
                  aria-hidden
                  className={cn("shrink-0 transition-transform duration-fast ease-soft", isOpen && "rotate-90")}
                  size={14}
                />
                {item.title}
              </button>
            </h3>
            <div
              className="animate-fade pb-3 pl-6 text-xs text-secondary"
              id={panelId}
              role="region"
              aria-labelledby={triggerId}
              hidden={!isOpen}
            >
              {item.children}
            </div>
          </section>
        );
      })}
    </div>
  );
}

/** Single-section accordion for simple disclosures (activity, advanced panels). */
export interface AccordionDisclosureProps {
  title: React.ReactNode;
  children: React.ReactNode;
  defaultOpen?: boolean;
  className?: string;
}

export function AccordionDisclosure({ title, children, defaultOpen = false, className }: AccordionDisclosureProps) {
  const [open, setOpen] = React.useState(defaultOpen);
  const contentId = React.useId();
  const triggerId = React.useId();

  return (
    <section className={cn("border-t border-hairline", className)}>
      <button
        id={triggerId}
        aria-controls={contentId}
        aria-expanded={open}
        className="touch-target press flex w-full items-center gap-2 rounded-md bg-transparent px-1 py-3 text-left text-xs font-medium text-secondary transition-colors duration-fast ease-soft hover:bg-hover hover:text-foreground"
        type="button"
        onClick={() => setOpen((current) => !current)}
      >
        <ChevronRight
          aria-hidden
          className={cn("shrink-0 transition-transform duration-fast", open && "rotate-90")}
          size={14}
        />
        {title}
      </button>
      <div
        className="animate-fade pb-3 pl-6 text-xs text-secondary"
        id={contentId}
        role="region"
        aria-labelledby={triggerId}
        hidden={!open}
      >
        {children}
      </div>
    </section>
  );
}
