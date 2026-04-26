(function () {
  var lb = document.createElement('div');
  lb.className = 'lightbox';
  lb.innerHTML =
    '<div class="lightbox-toolbar">' +
    '<a class="lightbox-original" target="_blank" rel="noopener">full resolution</a>' +
    '<button class="lightbox-close">close</button>' +
    '</div>' +
    '<button class="lightbox-prev" aria-label="Previous">‹</button>' +
    '<div class="lightbox-inner">' +
    '<img class="lightbox-img" alt="">' +
    '<div class="lightbox-exif">' +
    '<span class="lightbox-exif-time"></span>' +
    '<span class="lightbox-exif-meta"></span>' +
    '</div>' +
    '</div>' +
    '<button class="lightbox-next" aria-label="Next">›</button>';
  document.body.appendChild(lb);

  var img = lb.querySelector('.lightbox-img');
  var original = lb.querySelector('.lightbox-original');
  var exifMeta = lb.querySelector('.lightbox-exif-meta');
  var exifTime = lb.querySelector('.lightbox-exif-time');
  var items = [];
  var idx = 0;

  function open(i) {
    idx = (i + items.length) % items.length;
    img.src = items[idx].dataset.medium || items[idx].href;
    original.href = items[idx].href;

    var cameraLens = items[idx].dataset.exifCameraLens || '';
    var tech = items[idx].dataset.exifTech || '';
    exifMeta.textContent = [cameraLens, tech].filter(Boolean).join(' · ');
    var datetime = items[idx].dataset.exifDatetime || '';
    exifTime.textContent = datetime ? 'Photo taken: ' + datetime : '';

    lb.classList.add('is-open');
    document.body.style.overflow = 'hidden';
  }

  function close() {
    lb.classList.remove('is-open');
    document.body.style.overflow = '';
    img.src = '';
  }

  lb.addEventListener('click', function (e) {
    if (e.target.classList.contains('lightbox-original')) return;
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
