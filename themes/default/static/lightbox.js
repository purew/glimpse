(function () {
  var lb = document.createElement('div');
  lb.className = 'lightbox';
  lb.innerHTML =
    '<button class="lightbox-close" aria-label="Close">×</button>' +
    '<button class="lightbox-prev" aria-label="Previous">‹</button>' +
    '<img class="lightbox-img" alt="">' +
    '<button class="lightbox-next" aria-label="Next">›</button>';
  document.body.appendChild(lb);

  var img = lb.querySelector('.lightbox-img');
  var items = [];
  var idx = 0;

  function open(i) {
    idx = (i + items.length) % items.length;
    img.src = items[idx].dataset.medium || items[idx].href;
    lb.classList.add('is-open');
    document.body.style.overflow = 'hidden';
  }

  function close() {
    lb.classList.remove('is-open');
    document.body.style.overflow = '';
    img.src = '';
  }

  lb.addEventListener('click', function (e) {
    if (e.target === lb || e.target.classList.contains('lightbox-close')) {
      close();
    } else if (e.target.classList.contains('lightbox-prev')) {
      open(idx - 1);
    } else if (e.target.classList.contains('lightbox-next')) {
      open(idx + 1);
    }
  });

  document.addEventListener('keydown', function (e) {
    if (!lb.classList.contains('is-open')) return;
    if (e.key === 'Escape') close();
    else if (e.key === 'ArrowLeft') open(idx - 1);
    else if (e.key === 'ArrowRight') open(idx + 1);
  });

  document.addEventListener('click', function (e) {
    var a = e.target.closest('[data-lightbox]');
    if (!a) return;
    e.preventDefault();
    items = Array.from(document.querySelectorAll('[data-lightbox]'));
    open(items.indexOf(a));
  });
})();
