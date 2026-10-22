import { render, screen } from '@testing-library/react'
import { expect, test } from 'vitest'
import { Markdown } from './Markdown'

test("a message's Markdown renders, and its HTML and scripts don't", () => {
  const { container } = render(
    <Markdown
      text={[
        '**Done.** See `serve_blob` and [the docs](https://example.com).',
        '',
        '<script>window.hacked = true</script><img src=x onerror="window.hacked = true">',
        '',
        '| a | b |',
        '|---|---|',
        '| 1 | 2 |',
        '',
        '```rust',
        'fn main() {}',
        '```',
        '',
        '[bad](javascript:alert(1))',
      ].join('\n')}
    />,
  )
  expect(screen.getByText('Done.').tagName).toBe('STRONG')
  expect(screen.getByText('serve_blob').tagName).toBe('CODE')
  expect(screen.getByRole('link', { name: 'the docs' })).toHaveAttribute('rel', 'noreferrer noopener')
  expect(screen.getByRole('table')).toBeInTheDocument()
  expect(screen.getByText('rust')).toBeInTheDocument()
  expect(container.querySelector('script')).toBeNull()
  expect(container.querySelector('img')).toBeNull()
  expect(screen.getByText('bad').getAttribute('href') ?? '').not.toContain('javascript')
  expect((window as unknown as { hacked?: boolean }).hacked).toBeUndefined()
})
