import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { Site } from "../src/site.tsx";

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.colorScheme;
});

describe("project website", () => {
  test("the color-scheme control switches the complete page between dark and light", () => {
    render(<Site />);

    const toggle = screen.getByRole("button", { name: "Switch color scheme" });
    expect(document.documentElement.dataset.colorScheme).toBe("dark");

    fireEvent.click(toggle);
    expect(document.documentElement.dataset.colorScheme).toBe("light");

    fireEvent.click(toggle);
    expect(document.documentElement.dataset.colorScheme).toBe("dark");
  });

  test("the Release-room carousel leads the page before the autoplaying film", () => {
    const { container } = render(<Site videoUrl="https://example.test/film.mp4" />);

    const sections = [...container.querySelectorAll("main > section")];
    expect(sections.slice(0, 2).map((section) => section.id)).toEqual(["showcase", "film"]);

    const stage = screen.getByTestId("showcase-stage");
    expect(stage.dataset.slide).toBe("0");
    fireEvent.click(screen.getByRole("button", { name: "Next workspace" }));
    expect(stage.dataset.slide).toBe("1");

    const video = container.querySelector("video");
    expect(video?.autoplay).toBe(true);
    expect(video?.muted).toBe(true);
    expect(video?.loop).toBe(true);
    expect(video?.controls).toBe(true);
  });
});
