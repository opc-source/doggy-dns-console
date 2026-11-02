import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import MarkdownBlock from '@/components/MarkdownBlock.vue'

function render(text: string) {
  return mount(MarkdownBlock, { props: { text } })
}

describe('MarkdownBlock', () => {
  it('turns markdown headings, lists and code fences into elements', () => {
    const wrapper = render(
      ['## 失败分组', '', '- SERVFAIL 12 次', '- REFUSED 3 次', '', '```toml', 'key = 1', '```'].join('\n'),
    )

    expect(wrapper.find('h2').text()).toBe('失败分组')
    expect(wrapper.findAll('li')).toHaveLength(2)
    expect(wrapper.find('pre code').text()).toContain('key = 1')
  })

  it('renders an indented block when the whole answer is JSON', () => {
    const wrapper = render('{"verdict":"ok","confidence":0.9}')

    expect(wrapper.find('pre').exists()).toBe(true)
    expect(wrapper.find('pre').text()).toBe('{\n  "verdict": "ok",\n  "confidence": 0.9\n}')
    expect(wrapper.find('h2').exists()).toBe(false)
  })

  it('keeps every row of a markdown table', () => {
    const wrapper = render(
      ['| 域名 | 失败 |', '| --- | --- |', '| user-service | 12 |', '| payment-service | 7 |'].join('\n'),
    )

    const rows = wrapper.findAll('tbody tr')
    expect(rows).toHaveLength(2)
    expect(rows[0]?.findAll('td').map((cell) => cell.text())).toEqual(['user-service', '12'])
  })

  it('drops scripts, event-handler tags and javascript links from model output', () => {
    const wrapper = render(
      ['<script>alert(1)</script>', '<img src=x onerror=alert(1)>', '[点我](javascript:alert(1))', '正文'].join('\n\n'),
    )

    expect(wrapper.element.querySelector('script')).toBeNull()
    expect(wrapper.element.querySelector('img')).toBeNull()
    for (const link of wrapper.findAll('a')) {
      expect(link.attributes('href')).toBeUndefined()
    }
    expect(wrapper.text()).toContain('正文')
  })

  it('keeps http links but makes them non-opener', () => {
    const wrapper = render('[OpenObserve](https://example.com/dash)')

    const link = wrapper.find('a')
    expect(link.attributes('href')).toBe('https://example.com/dash')
    expect(link.attributes('target')).toBe('_blank')
    expect(link.attributes('rel')).toBe('noopener noreferrer')
  })
})
