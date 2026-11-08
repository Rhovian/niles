import DOMPurify from 'dompurify';
import { marked } from 'marked';
import { memo } from 'preact/compat';

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  const html = DOMPurify.sanitize(marked.parse(text, { async: false }), {
    USE_PROFILES: { html: true }, RETURN_DOM_FRAGMENT: true,
    FORBID_TAGS: ['form', 'input', 'button', 'textarea', 'select', 'style', 'img'],
    FORBID_ATTR: ['style', 'id', 'name'],
  });
  for (const link of html.querySelectorAll('a[href]')) {
    link.setAttribute('target', '_blank');
    link.setAttribute('rel', 'noopener noreferrer');
  }
  const container = document.createElement('div');
  container.append(html);
  return <div class="markdown" dangerouslySetInnerHTML={{ __html: container.innerHTML }} />;
});
