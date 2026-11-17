import type { SVGAttributes } from "react";

export interface XFalconMarkProps extends SVGAttributes<SVGSVGElement> {
  label?: string;
}

export function XFalconMark({ label, ...props }: XFalconMarkProps) {
  return (
    <svg
      viewBox="0 0 1254 950"
      role={label ? "img" : undefined}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      focusable="false"
      {...props}
    >
      <path
        fill="currentColor"
        fillRule="evenodd"
        d="M 705 480 L 620 591 L 683 656 L 776 534 L 1020 454 L 945 600 L 864 622 L 892 566 L 815 590 L 730 704 L 922 906 L 830 906 L 509 572 L 550 520 L 575 543 L 688 393 L 1124 249 L 1053 387 L 968 413 L 994 363 L 730 448 Z M 594 372 L 383 645 L 217 471 L 88 471 L 45 512 L 45 407 L 230 250 L 531 250 L 479 318 L 251 318 L 151 404 L 247 406 L 344 509 L 380 542 L 601 253 L 1226 45 L 1158 180 L 1072 208 L 1095 158 L 680 295 L 642 308 Z M 458 639 L 466 633 L 729 906 L 638 906 L 424 685 Z"
      />
    </svg>
  );
}
