import { expect, it, onTestFinished, vi } from 'vitest';
import { render } from '@testing-library/preact';
import { useEffect } from 'preact/hooks';

it('finishes passive effect cleanup before leaving the test environment', () => {
  const dispose = vi.fn();
  function Component() {
    useEffect(() => dispose, []);
    return null;
  }
  render(<Component />);
  onTestFinished(() => expect(dispose).toHaveBeenCalledOnce());
});
