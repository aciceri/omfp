/** `omfp:jobs`: the declared jobs of a workspace (`.omfp/jobs.json`), their runs, and Run, Stop, Enable and Disable
 * (UI-742, UI-743). Copyable (AGT-021, UI-749): value imports only from `react`, `omfp/sdk`, `omfp/kit` and this folder;
 * its data through `useJobs` (AGT-090). */

import { JobsWidget } from "./jobs.tsx";

export { JobsWidget };
export default JobsWidget;
