import DOMPurify from 'dompurify'
import { marked } from 'marked'

// Registered once, at module load. `addHook` appends to the DOMPurify singleton,
// so calling it from a component setup would pile up one hook per mounted block.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.nodeName !== 'A' || !node.hasAttribute('href')) return
  // A mailto belongs to the mail client; opening one in a tab shows a blank page.
  if (/^mailto:/i.test(node.getAttribute('href') ?? '')) return
  node.setAttribute('target', '_blank')
  node.setAttribute('rel', 'noopener noreferrer')
})

const ALLOWED_TAGS = [
  'a', 'blockquote', 'br', 'code', 'del', 'em', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
  'hr', 'li', 'ol', 'p', 'pre', 'strong', 'table', 'tbody', 'td', 'th', 'thead', 'tr', 'ul',
]

export function sanitizeMarkdown(text: string): string {
  return DOMPurify.sanitize(String(marked.parse(text, { gfm: true, breaks: true })), {
    ALLOWED_TAGS,
    ALLOWED_ATTR: ['href', 'title', 'target', 'rel'],
    // The default URI policy also admits tel:, sms:, ftp: and relative links.
    ALLOWED_URI_REGEXP: /^(?:https?:|mailto:)/i,
    ALLOW_DATA_ATTR: false,
  })
}
