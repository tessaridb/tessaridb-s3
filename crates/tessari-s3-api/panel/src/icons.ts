//! The console's icons: one outline family on a 24px grid — stroke 2, round
//! caps and joins, `currentColor` so they follow the theme and the state they
//! sit in. Every icon here sits beside a text label, so each is hidden from
//! screen readers; an icon never carries meaning on its own.

const SVG = "http://www.w3.org/2000/svg";

/** A rounded rectangle as a path. */
const box = (x: number, y: number, w: number, h: number, r: number): string =>
  `M${x + r} ${y}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}v${h - 2 * r}a${r} ${r} 0 0 1 -${r} ${r}h-${w - 2 * r}a${r} ${r} 0 0 1 -${r} -${r}v-${h - 2 * r}a${r} ${r} 0 0 1 ${r} -${r}z`;

/** A circle as a path. */
const ring = (cx: number, cy: number, r: number): string => `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 -${2 * r} 0`;

const SHAPES = {
  overview: [box(3, 3, 7, 7, 1.5), box(14, 3, 7, 7, 1.5), box(3, 14, 7, 7, 1.5), box(14, 14, 7, 7, 1.5)],
  buckets: ["M4 6c0 1.66 3.58 3 8 3s8-1.34 8-3-3.58-3-8-3-8 1.34-8 3z", "M4 6v12c0 1.66 3.58 3 8 3s8-1.34 8-3V6", "M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"],
  record: ["M9 6h11", "M9 12h11", "M9 18h11", "M4.5 6h.01", "M4.5 12h.01", "M4.5 18h.01"],
  sun: [ring(12, 12, 4), "M12 2.5v2", "M12 19.5v2", "M2.5 12h2", "M19.5 12h2", "M5.3 5.3l1.4 1.4", "M17.3 17.3l1.4 1.4", "M5.3 18.7l1.4-1.4", "M17.3 6.7l1.4-1.4"],
  moon: ["M20 14.5A8.5 8.5 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z"],
  "sign-out": ["M10 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h4", "M15 8l4 4-4 4", "M19 12H9"],
  folder: ["M3 7.5a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"],
  file: ["M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z", "M14 3v5h5"],
  download: ["M12 4v11", "M7.5 10.5L12 15l4.5-4.5", "M5 20h14"],
  trash: ["M4 7h16", "M9.5 7V4.5h5V7", "M6.5 7l1 13h9l1-13", "M10 11v5.5", "M14 11v5.5"],
  plus: ["M12 5v14", "M5 12h14"],
  users: [ring(9, 8, 3.5), "M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6", "M16 4.5a3.5 3.5 0 0 1 0 7", "M18 14c2 .6 3 2.8 3 6"],
  chevron: ["M9.5 6l6 6-6 6"],
  node: [box(3, 4, 18, 7, 2), box(3, 13, 18, 7, 2), "M7 7.5h.01", "M7 16.5h.01"],
  members: [ring(6, 7, 2.5), ring(18, 7, 2.5), ring(12, 18, 2.5), "M8.5 7h7", "M7.3 9.2l3.4 6.6", "M16.7 9.2l-3.4 6.6"],
  layers: ["M12 3l9 5-9 5-9-5z", "M3 13l9 5 9-5"],
  pulse: ["M3 12h4l3-7 4 14 3-7h4"],
  back: ["M14.5 6l-6 6 6 6"],
  disk: [box(3, 5, 18, 14, 2.5), "M3 13h18", "M16.5 16h.01"],
} as const;

export type IconName = keyof typeof SHAPES;

/** The icon `name`, decorative: hidden from assistive technology. */
export function icon(name: IconName): SVGSVGElement {
  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("class", "icon");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("focusable", "false");
  svg.setAttribute("fill", "none");
  svg.setAttribute("stroke", "currentColor");
  svg.setAttribute("stroke-width", "2");
  svg.setAttribute("stroke-linecap", "round");
  svg.setAttribute("stroke-linejoin", "round");
  for (const d of SHAPES[name]) {
    const path = document.createElementNS(SVG, "path");
    path.setAttribute("d", d);
    svg.append(path);
  }
  return svg;
}
