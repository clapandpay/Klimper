export default function Header({ title }: { title: string }) {
  return <h1 className='px-6 pb-4 text-3xl font-extrabold tracking-tight'>{title}</h1>;
}
