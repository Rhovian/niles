import { expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/preact';
import { Wake } from './Wake';

it('sends wake to the relay, prevents concurrent sends and waits for host recovery', async () => {
  let complete!: (response: Response) => void;
  const fetcher = vi.fn(() => new Promise<Response>(resolve => { complete = resolve; }));
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
it.each([403, 404, 500])('shows wake HTTP %s failures and permits retry', async status => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ status }));
  const refresh = vi.fn();
  render(<Wake host={{ id: 'desk', name: 'Desktop' }} refresh={refresh} />);
  const button = screen.getByRole('button', { name: 'Wake Desktop' });
  fireEvent.click(button);
  expect((await screen.findByRole('alert')).textContent).toContain(String(status));
  await waitFor(() => expect(button.hasAttribute('disabled')).toBe(false));
  expect(refresh).not.toHaveBeenCalled();
});
it('reports network failures and disables wake without a configured host ID', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Network down')));
  const app = render(<Wake host={{ id: 'desk', name: 'Desktop' }} refresh={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Wake Desktop' }));
  expect((await screen.findByRole('alert')).textContent).toContain('Network down');
  app.unmount();
  render(<Wake host={{ id: '', name: 'This host' }} refresh={() => {}} />);
  expect(screen.getByRole('button').hasAttribute('disabled')).toBe(true);
});
