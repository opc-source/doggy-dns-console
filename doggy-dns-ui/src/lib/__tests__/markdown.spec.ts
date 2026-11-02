import { describe, expect, it } from 'vitest'

import { sanitizeMarkdown } from '@/lib/markdown'

// No component is mounted here: the sanitising rules, including the link hook,
// must come from the module itself rather than from a component instance.
describe('sanitizeMarkdown', () => {
  it('makes links non-opener without any component mounted', () => {
    const html = sanitizeMarkdown('[docs](https://example.com/a)')

    expect(html).toContain('href="https://example.com/a"')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
  })

  it('keeps only http, https and mailto targets', () => {
    expect(sanitizeMarkdown('[c](tel:+1234)')).not.toContain('href')
    expect(sanitizeMarkdown('[c](ftp://example.com/x)')).not.toContain('href')
    expect(sanitizeMarkdown('[c](mailto:ops@example.com)')).toContain('href="mailto:ops@example.com"')
  })

  it('opens a mailto in the mail client, not a blank tab', () => {
    const html = sanitizeMarkdown('[邮件](mailto:ops@example.com)')

    expect(html).toContain('href="mailto:ops@example.com"')
    expect(html).not.toContain('target')
  })

  it('closes the attribute allowlist against data attributes', () => {
    expect(sanitizeMarkdown('<p data-evil="1">正文</p>')).not.toContain('data-evil')
  })

  it('strips markup that could run', () => {
    expect(sanitizeMarkdown('<script>alert(1)</script>')).not.toContain('script')
    expect(sanitizeMarkdown('<img src=x onerror=alert(1)>')).not.toContain('img')
    expect(sanitizeMarkdown('<math><mi xlink:href="data:text/html,<script>a</script>">x</mi></math>')).not.toContain(
      'xlink:href',
    )
  })
})
