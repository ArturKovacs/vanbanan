
// I don't know if there is a nicer way to get the correct type for self, but this works okay
/** @type {ServiceWorkerGlobalScope} */
const selfSW = self;

selfSW.addEventListener('install', () => {
    console.log('Service Worker installed');
    selfSW.skipWaiting();
});


selfSW.addEventListener('push', (event) => {
    console.log('Push event received:', event);

    if (!event.data) {
        console.warn('Push event has no data');
        return;
    }

    const body = event.data.json();

    const floor = body.floor;
    const definiteArticle = floor === 1 ? 'az' : 'a'; // Floors go from 0 to 3, so this is enough

    selfSW.registration.showNotification('Van Banán?', {
        body: `Banánt láttak ${definiteArticle} ${floor}. emeleten!`,
        data: { floor }
    });
});


selfSW.addEventListener('activate', async (a) => {
    console.log('Service Worker activated!');

    // setInterval(async () => {
    //     console.log('Showing notification timeout');

    //     self.registration.showNotification('Hello Elm!', {
    //         body: 'Current time is: ' + new Date().toLocaleTimeString(),
    //     });

    // }, 10000);
});


selfSW.addEventListener('notificationclick', (event) => {
    event.notification.close();

    const clients = selfSW.clients;

    const { floor } = event.notification.data;
    const targetPath = `/floor/${floor}`;

    // Full disclosure: I stole this form MDN
    //
    // This looks to see if the current is already open and
    // focuses if it is
    event.waitUntil(
        clients
        .matchAll({
            type: "window",
        })
        .then((clientList) => {
            for (const client of clientList) {
                if (client.url === targetPath && "focus" in client) {
                    return client.focus();
                }
            }
            if (clients.openWindow) return clients.openWindow(targetPath);
        }),
    );
});
