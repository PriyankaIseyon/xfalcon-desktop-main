import ReactMarkdown, { type Components } from "react-markdown";
import { REHYPE_PLUGINS, REMARK_PLUGINS } from "../chat/markdownPipeline";
import { Link } from "../../ui/Link";

const RELEASE_NOTES_COMPONENTS: Components = {
  a({ href, children }) {
    return href ? <Link href={href}>{children}</Link> : <span>{children}</span>;
  },
  img({ alt }) {
    return <span className="text-secondary italic">Image omitted{alt ? `: ${alt}` : ""}</span>;
  },
  pre({ children }) {
    return <pre className="max-h-80 max-w-full overflow-auto overscroll-contain">{children}</pre>;
  },
  code({ className, children }) {
    return <code className={[className, "assistant-markdown__inline-code"].filter(Boolean).join(" ")}>{children}</code>;
  },
  table({ children }) {
    return (
      <div
        aria-label="Scrollable release notes table"
        className="assistant-markdown__table-scroll"
        role="region"
        tabIndex={0}
      >
        <table>{children}</table>
      </div>
    );
  },
};

/** Render updater documentation as passive Markdown, without assistant preview actions. */
export function ReleaseNotesMarkdown({ source }: { source: string }) {
  return (
    <div className="min-w-0 max-w-full overflow-hidden rounded-lg border border-default bg-card px-4 py-2">
      <div className="assistant-markdown type-body min-w-0 max-w-full break-words">
        <ReactMarkdown
          components={RELEASE_NOTES_COMPONENTS}
          rehypePlugins={REHYPE_PLUGINS}
          remarkPlugins={REMARK_PLUGINS}
        >
          {source}
        </ReactMarkdown>
      </div>
    </div>
  );
}
