/** The composer's attached images (UI-060). */
import { IconButton } from "omfp/kit";
import { type Draft, strings } from "omfp/sdk";

const text = strings.chat.composer;

/** Thumbnails of the attached images, each removable unless `locked` (the message on its way). */
export function Attachments({
  draft,
  setDraft,
  locked = false,
}: {
  draft: Draft;
  setDraft: (update: (previous: Draft) => Draft) => void;
  locked?: boolean;
}) {
  return (
    <ul className="omfp-composer__attachments" aria-label={text.attachments}>
      {draft.images.map((image, index) => (
        // Attachments have no identity besides their position.
        // biome-ignore lint/suspicious/noArrayIndexKey: see above
        <li key={index} className="omfp-composer__attachment" data-testid="attachment">
          <img src={`data:${image.mimeType ?? ""};base64,${image.data ?? ""}`} alt={text.attachment(index + 1)} />
          <IconButton
            label={text.removeAttachment(index + 1)}
            icon="×"
            data-testid="attachment-remove"
            isDisabled={locked}
            onPress={() =>
              setDraft((previous) => ({
                ...previous,
                images: previous.images.filter((_, position) => position !== index),
              }))
            }
          />
        </li>
      ))}
    </ul>
  );
}
