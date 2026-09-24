// Development builds only: reload the page when the server restarts with a new build.
(() => {
  let known = null;
  const check = async () => {
    try {
      const response = await fetch("/dev/boot-id", { cache: "no-store" });
      const id = await response.text();
      if (known !== null && id !== known) {
        location.reload();
        return;
      }
      known = id;
    } catch {
      // The server is restarting; try again.
    }
    setTimeout(check, 1000);
  };
  check();
})();
