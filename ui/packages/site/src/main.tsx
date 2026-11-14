import "@omfp/kit/tokens.css";
import "@omfp/kit/themes.css";
import "@omfp/kit/kit.css";
import "./styles.css";

import { createRoot } from "react-dom/client";
import { Site } from "./site.tsx";

declare const __OMFP_SITE_VIDEO_URL__: string;

const container = document.getElementById("root");
if (container === null) {
  throw new Error("index.html has no #root");
}

const videoUrl = __OMFP_SITE_VIDEO_URL__.trim();
createRoot(container).render(<Site {...(videoUrl === "" ? {} : { videoUrl })} />);
