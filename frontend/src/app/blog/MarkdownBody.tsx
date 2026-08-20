import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

/**
 * Renders a post body, which is stored as markdown source everywhere — in the database, in
 * the API response, in the editor's textarea, and in what search matches against. Rendering
 * happens here and only here, so there is exactly one representation of a post to keep
 * consistent.
 *
 * `react-markdown` builds a React element tree rather than setting `innerHTML`, so raw HTML
 * inside a post is escaped and shown as text instead of executing. That is why there's no
 * sanitizer in this stack. It also means **`rehype-raw` must not be added** without deciding
 * to accept HTML injection — it exists precisely to switch that protection off.
 *
 * `remark-gfm` adds the GitHub extensions people actually type: tables, strikethrough, task
 * lists, and bare-URL autolinking.
 *
 * The `markdown-body` class is not decoration. Tailwind's preflight resets headings, lists,
 * and blockquotes to unstyled, so without the rules in `globals.css` a rendered post is
 * nearly indistinguishable from the plain text this replaced.
 */
export function MarkdownBody({ children }: { children: string }) {
  return (
    <div className="markdown-body">
      <Markdown remarkPlugins={[remarkGfm]}>{children}</Markdown>
    </div>
  );
}
