// SPDX-License-Identifier: AGPL-3.0-or-later
// Stand-in for Sage's layout in the approvals preview (vite.preview.config.ts).
export default function Layout({ children }: { children: React.ReactNode }) {
  return <div className='min-h-screen py-6'>{children}</div>;
}
