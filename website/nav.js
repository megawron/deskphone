// Conditionally load the Popover API polyfill only when native support is missing
if (!Object.hasOwn(HTMLElement.prototype, "popover")) {
  import("https://unpkg.com/@oddbird/popover-polyfill@latest/dist/popover.min.js");
}

const header = document.querySelector(".site-header");
const inner = document.querySelector(".header-inner");
const menu = document.getElementById("site-menu");

if (header && inner && menu) {
  let isWide = false;
  let lastHeaderWidth = 0;

  const isMenuOpen = () => {
    try {
      if (menu.matches(":popover-open")) return true;
    } catch {
      // Fallback for browsers without native :popover-open selector
    }
    return menu.classList.contains(":popover-open");
  };

  function syncNavLayout() {
    const currentWidth = header.getBoundingClientRect().width;

    // While collapsed, only remeasure when the header has grown to avoid layout thrashing
    if (!isWide && currentWidth <= lastHeaderWidth && lastHeaderWidth > 0) {
      lastHeaderWidth = currentWidth;
      return;
    }
    lastHeaderWidth = currentWidth;

    const wasOpen = !isWide && isMenuOpen();

    // Temporarily disable transitions during measurement to prevent flicker
    header.classList.add("is-measuring");
    header.classList.add("is-wide");
    menu.removeAttribute("popover");

    const fitsInline = inner.scrollWidth <= inner.clientWidth;

    if (fitsInline) {
      isWide = true;
    } else {
      isWide = false;
      header.classList.remove("is-wide");
      menu.setAttribute("popover", "auto");
      if (wasOpen && typeof menu.showPopover === "function") {
        try {
          menu.showPopover();
        } catch {
          // Ignore if polyfill has not finished initializing yet
        }
      }
    }

    requestAnimationFrame(() => {
      header.classList.remove("is-measuring");
    });
  }

  // Close the mobile popover menu when selecting an in-page anchor link
  menu.addEventListener("click", (event) => {
    const link = event.target.closest('a[href^="#"]');
    if (link && !isWide && isMenuOpen() && typeof menu.hidePopover === "function") {
      menu.hidePopover();
    }
  });

  const observer = new ResizeObserver(syncNavLayout);
  observer.observe(header);
  observer.observe(menu);

  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(() => {
      lastHeaderWidth = 0;
      syncNavLayout();
    });
  }

  syncNavLayout();
}
