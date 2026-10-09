// The command text, links, and navigation are complete without JavaScript.
// Only reveal copy controls when the secure-context Clipboard API is present.
if (window.isSecureContext && navigator.clipboard?.writeText) {
  for (const button of document.querySelectorAll('[data-copy-target]')) {
    const command = document.getElementById(button.dataset.copyTarget);
    const status = button.closest('.install-option')?.querySelector('.copy-status');
    if (!command || !status) continue;

    button.hidden = false;
    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(command.textContent.trim());
        status.textContent = button.dataset.copySuccess;
      } catch {
        status.textContent = button.dataset.copyFailure;
      }
    });
  }
}
