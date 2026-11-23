/**
 * The file picker (UI-117): when a path that does not exist names several files of the workspace
 * (`workspaces.find_file`, DMN-092), the frame offers them in the style of the `:` line — an input filtering the
 * candidates, the arrows moving through them (the best one highlighted), Enter opening the highlighted one in place
 * of the missing path's tab, Escape (or leaving the line) cancelling the opening.
 */

import { type ReactNode, useState } from "react";
import { cancelFilePicker, chooseFile, FILE_PICKER, type FilePicker, useHost, useWindowState } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { Picker } from "./picker.tsx";

const text = strings.shell.files;

export function FilePickerLine(): ReactNode {
  const [picker] = useWindowState(FILE_PICKER);
  // A new picker starts afresh (its filter and highlight).
  return picker === undefined ? null : <PickerInput key={`${picker.workspaceId}:${picker.path}`} picker={picker} />;
}

function PickerInput({ picker }: { picker: FilePicker }): ReactNode {
  const host = useHost();
  const [query, setQuery] = useState("");
  const typed = query.trim().toLowerCase();
  return (
    <Picker
      id="file-picker"
      title={text.picker(picker.path)}
      empty={text.noMatch}
      prompt={
        <span className="omfp-file-picker__prompt" aria-hidden="true">
          {text.pickerPrompt} {picker.path}
        </span>
      }
      options={picker.candidates
        .filter((candidate) => candidate.path.toLowerCase().includes(typed))
        .map((candidate) => ({
          id: candidate.path,
          title: candidate.path,
          description: [text[candidate.match], ...(candidate.mentioned ? [text.mentioned] : [])].join(", "),
        }))}
      search={setQuery}
      choose={(path) => chooseFile(host, picker.workspaceId, picker.path, path, picker.options)}
      cancel={() => cancelFilePicker(host)}
    />
  );
}
