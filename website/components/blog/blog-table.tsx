import type { ComponentProps } from "react";

/** Keep table layout (`width: 100%`) while allowing wide multi-column tables to scroll. */
export function BlogTable(props: ComponentProps<"table">) {
  return (
    <div className="blog-table-scroll">
      <table {...props} />
    </div>
  );
}
