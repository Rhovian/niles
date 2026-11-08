import { deferred } from './test-utils';
import { expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { Wake } from './Wake';

it('sends wake to the relay, prevents concurrent sends and waits for host recovery', async () => {
  const { promise, resolve: complete } = deferred<Response>();
  const fetcher = vi.fn(() => promise);
  vi.stubGlobal('fetch', fetcher);
  const refresh = vi.fn();
  render(<Wake host={{ id: 'desk', name: 'Desktop' }} refresh={refresh} />);
  const button = screen.getByRole('button', { name: 'Wake Desktop' });
  fireEvent.click(button);
  expect(button.hasAttribute('disabled')).toBe(true);
  expect(fetcher).toHaveBeenCalledWith('/api/hosts/desk/wake', { method: 'POST', cache: 'no-store', headers: { 'Content-Type': 'application/json' }, body: 'null' });
  expect(refresh).not.toHaveBeenCalled();
  complete({ status: 204 } as Response);
  await screen.findByText('Wake packet sent. Waiting for host…');
  expect(refresh).toHaveBeenCalledOnce();
  expect(button.hasAttribute('disabled')).toBe(false);
});
it.each([403, 404, 500, 'Network down'])('shows wake failure %s and permits retry', async failure => {
  vi.stubGlobal('fetch', typeof failure === 'number' ? vi.fn().mockResolvedValue({ status: failure }) : vi.fn().mockRejectedValue(new TypeError(failure)));
  const refresh = vi.fn();
  render(<Wake host={{ id: 'desk', name: 'Desktop' }} refresh={refresh} />);
  const button = screen.getByRole('button', { name: 'Wake Desktop' });
  fireEvent.click(button);
  expect((await screen.findByRole('alert')).textContent).toContain(String(failure));
  await waitFor(() => expect(button.hasAttribute('disabled')).toBe(false));
  expect(refresh).not.toHaveBeenCalled();
});
it('disables wake without a configured host ID', () => {
  render(<Wake host={{ id: '', name: 'This host' }} refresh={() => {}} />);
  expect(screen.getByRole('button').hasAttribute('disabled')).toBe(true);
});
