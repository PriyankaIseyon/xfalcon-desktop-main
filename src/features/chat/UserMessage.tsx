import { Paperclip } from "lucide-react";
import { memo } from "react";

import type { ChatAttachmentView } from "../../lib/generated/contracts";
import { TruncatedText } from "../../ui/TruncatedText";

export type UserMessageProps = {
  text: string;
  /** The files sent with this turn, named under its text. */
  attachments?: ChatAttachmentView[];
};

// Rendered as a JSX text child, never as markup: React escapes it on the way
// into the DOM. Escaping it here as well would put the entities themselves on
// screen — "don't" as "don&#39;t".
export const UserMessage = memo(function UserMessage({ text, attachments = [] }: UserMessageProps) {
  return (
    <>
      <p className="whitespace-pre-wrap">{text}</p>
      {attachments.length > 0 ? (
        <ul aria-label="Attached files" className="mt-2 flex flex-wrap gap-1.5">
          {attachments.map((attachment) => (
            <li key={attachment.attachment_id} className="max-w-[14rem]">
              <span className="inline-flex max-w-[14rem] items-center gap-1 rounded-pill border border-default px-2 py-1 text-xs text-secondary">
                <Paperclip aria-hidden className="shrink-0" size={12} />
                <TruncatedText className="min-w-0">{attachment.file_name}</TruncatedText>
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </>
  );
});
