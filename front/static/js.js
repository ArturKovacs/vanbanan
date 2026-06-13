// @ts-check

"use strict";

/**
 * This script starts the Elm application and communicates with it.
 * 
 * This script is needed to handle everything that cannot be done directly
 * within Elm, for example service worker registration and notification handling.
 */ 

const Elm = /** @type {any} */ (window).Elm;

/**
 * @param {number} ms 
 * @returns 
 */
function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

const PORT_RESULT_OK_NAME = "ok"
const PORT_RESULT_FAILED_NAME = "failed"

/**
 * @param {string} message 
 */
function displayFatalError(message) {
    const appElement = document.querySelector("body div");
    if (appElement === null) {
        console.error('Not found "app" element, while trying to display', message);
        return;
    }
    if (!(appElement instanceof HTMLElement)) {
        console.error('The app element was not an instance of HTMLElement. While trying to display', message);
        return;
    }
    
    appElement.replaceChildren();

    appElement.textContent = message;
    appElement.style.padding = "20px";
    appElement.style.boxSizing = "border-box";
    appElement.style.width = "100%";
    appElement.style.textAlign = "center";
    appElement.style.fontSize = "20px";
}

if ('serviceWorker' in navigator) {
    navigator.serviceWorker.register('/serviceWorker.js').then(async (registration) => {
        try {
            await main(registration);
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            displayFatalError(`An error occurred while starting the application. Error: ${message}`);
        }
    }).catch(error => {
        // Using console.error here so that debug.js can process this. (instead of letting the browser handle the error)
        console.error('Failed to register service worker:', error);
    });
} else {
    displayFatalError("This website requires service worker support, but it seems that your browser does not support service workers.");

    console.warn('Service workers are not supported in this browser.');
}

// app.ports.triggerNotification.subscribe(async function () {    
// });

/**
 * @param {ArrayBuffer} arrayBuffer contains the key data
 * @returns {string} A base64url encoded string
 */
function toPushApiCompatibleBase64(arrayBuffer) {
    return new Uint8Array(arrayBuffer)
        .toBase64({ alphabet: "base64url" })
        .replace(/=+$/, ''); // Remove any trailing '=' characters used for padding
}

/**
 * @param {PushSubscription} subscription 
 * @returns {{endpoint: string, keys: { auth: string, p256dh: string }}} A JSON string representing the subscription
 */
function subscriptionToSerializable(subscription) {
    const authKey = subscription.getKey('auth');
    const p256dhKey = subscription.getKey('p256dh');
    if (!authKey || !p256dhKey) {
        throw new Error('Push subscription is missing required keys (auth or p256dh).');
    }
    if (!subscription.endpoint) {
        throw new Error('Push subscription is missing endpoint.');
    }
    const auth = toPushApiCompatibleBase64(authKey);
    const p256dh = toPushApiCompatibleBase64(p256dhKey);

    return {
        endpoint: subscription.endpoint,
        keys: {
            auth: auth,
            p256dh: p256dh
        }
    };
}


/**
 * @param {PushSubscription} subscription 
 * @param {number} floor
 */
async function sendSubscriptionToServer(subscription, floor) {
    const subscriptionBody = {
        subscription_info: subscriptionToSerializable(subscription),
        floor: floor
    };
    const subscriptionResponse = await fetch('/api/subscription', {
        method: "POST",
        headers: {
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(subscriptionBody)
    });

    if (!subscriptionResponse.ok) {
        let errorMessage = `Failed to send subscription operation to server.`;
        try {
            const errorText = await subscriptionResponse.json();
            errorMessage += ` Server sent: ${errorText}`;
        } catch (error) {
            console.info('Failed to parse error response from server as JSON. Error was:', error);
        }
        throw new Error(errorMessage);
    }
}

/**
 * @param {PushSubscription} subscription 
 * @param {number} floor
 */
async function sendUnsubscribeToServer(subscription, floor) {
    const subscriptionDeleteInfo = {
        endpoint: subscription.endpoint,
        floor: floor.toString()
    };
    const queryParams = new URLSearchParams(subscriptionDeleteInfo);
    const subscriptionResponse = await fetch(`/api/subscription?${queryParams}`, {
        method: "DELETE",
    });

    if (!subscriptionResponse.ok) {
        let errorMessage = `Failed to send UNsubscribe operation to server.`;
        try {
            const errorText = await subscriptionResponse.json();
            errorMessage += ` Server sent: ${errorText}`;
        } catch (error) {
            console.info('Failed to parse error response from server as JSON. Error was:', error);
        }
        throw new Error(errorMessage);
    }
}

/**
 * @param {ServiceWorkerRegistration} serviceWorkerRegistration
 * @param {string} vapidPublicKey
 * @param {number[]} floors
 */
async function makeSubscription(serviceWorkerRegistration, vapidPublicKey, floors) {
    const subscription = await serviceWorkerRegistration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: vapidPublicKey
    });
    console.log('New push subscription created for floors:', floors, subscription);
    const ATTEMPTS = 3;
    for (const floor of floors) {
        for (let attempt = 1; attempt <= ATTEMPTS; attempt++) {
            try {
                await sendSubscriptionToServer(subscription, floor);
                return subscription;
            } catch (error) {
                console.error(`Attempt ${attempt} to resubscribe to push failed:`, error);
                if (attempt <= ATTEMPTS) {
                    console.log(`Retrying sending subscription in 1 second...`);
                    await sleep(1000);
                    continue;
                }
            }
        }
    }
    return subscription;
}

/**
 * @param {ArrayBuffer | string} applicationServerKey Must be from the PushSubscriptionOptions type
 * @param {string} vapidPublicKey a base64 encoded string representing the VAPID public key
 */
function isMatchingApplicationServerKey(applicationServerKey, vapidPublicKey) {
    if (applicationServerKey instanceof ArrayBuffer) {
        const applicationServerKeyBase64 = toPushApiCompatibleBase64(applicationServerKey);

        console.log('Comparing applicationServerKey (base64):', applicationServerKeyBase64, 'with vapidPublicKey:', vapidPublicKey);
        return applicationServerKeyBase64 === vapidPublicKey;
    } else if (applicationServerKey == vapidPublicKey) {
        return true;
    } else {
        console.warn('Unexpected type of applicationServerKey in push subscription:', applicationServerKey);
        return false;
    }
}

/**
 * 
 * @param {ServiceWorkerRegistration} serviceWorkerRegistration 
 */
async function main(serviceWorkerRegistration) {
    const publicKeyResponse = await fetch('/api/public-key');
    if (!publicKeyResponse.ok) {
        throw new Error(`Failed to fetch VAPID public key from server. Status was ${publicKeyResponse.status}`);
    }
    const vapidPublicKey = await publicKeyResponse.text();
    console.log('Fetched VAPID public key from server:', vapidPublicKey);

    /** 
     * We don't know if the VAPID key has changed on the server since the subscription was made,
     * so we need to unsubscribe and resubscribe to make sure we are using the correct VAPID key.
     * 
     * @param {PushSubscription} subscription
     * @param {number[]} floors
     */
    async function resubscribeToPush(subscription, floors) {
        try {
            await subscription.unsubscribe();
        } catch (error) {
            console.error('Failed to unsubscribe from push subscription during resubscription process:', error);
            // We can continue with the resubscription process even if unsubscribing failed, as it may have been a transient error
        }
        return await makeSubscription(serviceWorkerRegistration, vapidPublicKey, floors);
    }

    async function tryGetPushSubscription() {
        let subscription = await serviceWorkerRegistration.pushManager.getSubscription();
        console.log('serviceWorkerRegistration.pushManager.getSubscription:', subscription);
        /** @type {number[] | null} */
        let floors = null;

        /** @type {{subscription: PushSubscription, floors: number[]} | null} */
        let result = null;
        if (subscription) {
            const queryParams = new URLSearchParams({ endpoint: subscription.endpoint });
            const response = await fetch(`/api/subscription?${queryParams}`);
            const body = await response.json();
            floors = body.floors;
            if (!floors) {
                throw new Error(`Floors was falsy. This is unexpected. floors: ${floors}`);
            }
            // TODO get the floors for the subscription from the server and pass them to resubscribeToPush
            subscription = await resubscribeToPush(subscription, floors);
            result = {subscription, floors};
        }
        return result;
    }

    /** @param {number} floor */
    async function getOrMakePushSubscription(floor) {
        let subscription = await tryGetPushSubscription();
        if (!subscription) {
            const floors = [floor];
            let subscriptionDetails = await makeSubscription(serviceWorkerRegistration, vapidPublicKey, floors);
            subscription = {
                subscription: subscriptionDetails,
                floors: floors
            };
        }
        return subscription;
    }

    const subscription = await tryGetPushSubscription();

    const app = Elm.Main.init({
        node: document.getElementById("app"),
        flags: {
            subscribedToFloors: subscription?.floors ?? []
        }
    });

    app.ports.subscribeToFloor.subscribe(
        /** @param {number} targetFloor */
        async function (targetFloor) {
            console.log('Requesting notification permission and subscribing for notifications for floor', targetFloor);
            let resultName = PORT_RESULT_FAILED_NAME;
            try {
                const permission = await Notification.requestPermission();
                if (permission !== 'granted') {
                    console.warn('Notification permission was not granted. Permission is:', permission);
                    return;
                }
                try {
                    const subscription = await tryGetPushSubscription();
                    if (!subscription) {
                        await makeSubscription(serviceWorkerRegistration, vapidPublicKey, [targetFloor]);
                    } else {
                        await sendSubscriptionToServer(subscription.subscription, targetFloor);
                    }
                } catch (error) {
                    console.error('Service worker registration failed:', error);
                    throw error;
                }
                resultName = PORT_RESULT_OK_NAME;
            } catch (error) {
                resultName = PORT_RESULT_FAILED_NAME;
                console.error('An error occurred while requesting notification permission or registering service worker:', error);
            } finally {
                const result = {
                    name: resultName,
                    floor: targetFloor
                };
                app.ports.subscriptionResultHandler.send(result);
            }
        }
    );

    app.ports.unsubscribeFromFloor.subscribe(
        /** @param {number} targetFloor */
        async function (targetFloor) {
            let resultName = PORT_RESULT_FAILED_NAME
            try {
                const subscription = await tryGetPushSubscription();
                if (!subscription) {
                    console.error("No Push Manager Subscription found locally, when trying to unsubscribe from floor");
                    return;
                }
                await sendUnsubscribeToServer(subscription.subscription, targetFloor);
                resultName = PORT_RESULT_OK_NAME
            } finally {
                const result = {
                    name: resultName,
                    floor: targetFloor
                };
                app.ports.unsubscribeResultHandler.send(result)
            }
        }
    );
}
