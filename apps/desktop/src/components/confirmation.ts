/** 使用 WebView 内的可聚焦对话框；Escape、取消和宿主关闭都不提交操作。 */
export function confirmChange(root: HTMLElement, description: string, acceptLabel: string): Promise<boolean> {
  const doc = root.ownerDocument;
  const dialog = doc.createElement('dialog');
  dialog.setAttribute('aria-label', acceptLabel);
  const text = doc.createElement('p');
  text.textContent = description;
  const actions = doc.createElement('div');
  actions.className = 'actions';
  const cancel = doc.createElement('button');
  cancel.textContent = '取消';
  cancel.autofocus = true;
  const accept = doc.createElement('button');
  accept.textContent = acceptLabel;
  actions.append(cancel, accept);
  dialog.append(text, actions);
  root.append(dialog);

  return new Promise((resolve) => {
    const finish = (confirmed: boolean) => {
      dialog.close();
      dialog.remove();
      resolve(confirmed);
    };
    cancel.onclick = () => finish(false);
    accept.onclick = () => finish(true);
    dialog.oncancel = (event) => {
      event.preventDefault();
      finish(false);
    };
    dialog.showModal();
  });
}
