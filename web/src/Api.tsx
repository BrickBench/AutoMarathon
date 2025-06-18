export function formUrl(api: string) {
    if (window.location.protocol === 'https:') {

        return new URL(api, 'https://' + location.hostname);
    } else {
        return new URL(api, 'http://' + location.hostname + ":28010");
    }
    var base = window.location.protocol + "//" + window.location.hostname + ":28010";
    return new URL(api, base)
}

export async function doPost(api: string, method: string, object: any) {
    fetch(formUrl("/" + api), {
        method: method,
        headers: {
            'Access-Control-Allow-Origin': '*',
            'Content-Type': 'application/json'
        },
        body: JSON.stringify(object)
    });
}
