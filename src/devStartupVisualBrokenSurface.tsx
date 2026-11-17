// A component in its own file so `devStartupVisualEntry.tsx` mirrors
// `main.tsx`: an entry file that imports and renders, with no component
// definition of its own for `react-refresh/only-export-components` to flag.
export function BrokenSurface(): never {
  throw new Error("visual recovery fixture");
}
