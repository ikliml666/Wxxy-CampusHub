export function TodoPanel() {
  return (
    <section className="relative mx-auto mt-10 max-w-3xl rounded-[10px] border border-line bg-surface p-6">
      <span
        aria-hidden="true"
        className="absolute left-0 top-0 h-2 w-2 rounded-tl-[10px] bg-todo"
      />
      <h2 className="text-lg font-semibold text-text">待办</h2>
      <p className="mt-2 text-sm text-text-2">建设中</p>
    </section>
  );
}
