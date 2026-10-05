// SPDX-License-Identifier: AGPL-3.0-or-later
// Stand-in for Sage's layout in the approvals preview (vite.preview.config.ts).
export default function Header({ title }: { title: string }) {
  return (
    <h1 className='px-6 pb-4 text-3xl font-extrabold tracking-tight'>
      {title}
    </h1>
  );
}
