import { Shortcut } from "../ui/Kbd";
import { Modal } from "../ui/Modal";

export interface ShortcutHelpDialogProps {
  open: boolean;
  onClose: () => void;
}

interface ShortcutRow {
  keys: readonly string[];
  description: string;
  category: "Navigation" | "Composer" | "Views";
}

/** Canonical keys, never spellings: `Mod` becomes this platform's own
 * modifier. The sheet used to print `⌘/Ctrl` to everyone, which named a
 * key half its readers do not have. */
const SHORTCUTS: ShortcutRow[] = [
  { keys: ["Mod", "K"], description: "Search chats, projects, reports and settings", category: "Navigation" },
  { keys: ["Mod", "B"], description: "Toggle sidebar visibility", category: "Navigation" },
  { keys: ["Mod", "Shift", "F"], description: "Toggle Zen / Focus Mode", category: "Navigation" },
  { keys: ["Mod", "/"], description: "Open keyboard shortcuts cheat sheet", category: "Navigation" },
  { keys: ["@"], description: "Mention project files and MCP resources", category: "Composer" },
  { keys: ["/"], description: "Invoke slash commands (/clear, /model, /help)", category: "Composer" },
  { keys: ["Enter"], description: "Send message / Save in-place edit", category: "Composer" },
  { keys: ["Shift", "Enter"], description: "Insert newline in composer", category: "Composer" },
  { keys: ["Escape"], description: "Dismiss palette / Cancel in-place edit", category: "Composer" },
];

export function ShortcutHelpDialog({ open, onClose }: ShortcutHelpDialogProps) {
  return (
    <Modal open={open} onClose={onClose} size="lg" title="Keyboard Shortcuts">
      <div className="max-h-[60vh] overflow-y-auto space-y-1">
        <div className="divide-y divide-hairline">
          {SHORTCUTS.map((item, idx) => (
            <div key={idx} className="flex items-center justify-between gap-3 py-2.5 text-sm">
              {/* The same label-plus-trailing-glyph split `Menu` already uses:
                  the label gives way first, the glyph never does. Unmarked,
                  both are default-shrinkable flex children with no minimum,
                  so a narrow window (`minWidth: 900` still leaves ~600px once
                  the sidebar and dialog padding take their share) would widen
                  the row past its box instead of truncating the text -- the
                  one axis this sheet has no vertical scroll to absorb. */}
              <span className="min-w-0 flex-1 truncate text-secondary">{item.description}</span>
              <Shortcut className="shrink-0" keys={item.keys} />
            </div>
          ))}
        </div>
      </div>
    </Modal>
  );
}
