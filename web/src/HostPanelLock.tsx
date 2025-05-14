import { Button } from "react-bootstrap";
import { LockState, AuthState } from "./websocket";

function timeAgo(input : Date) {
    const date = (input instanceof Date) ? input : new Date(input);
    const formatter = new Intl.RelativeTimeFormat('en');
    const ranges = {
      years: 3600 * 24 * 365,
      months: 3600 * 24 * 30,
      weeks: 3600 * 24 * 7,
      days: 3600 * 24,
      hours: 3600,
      minutes: 60,
      seconds: 1
    };
    const secondsElapsed = (date.getTime() - Date.now()) / 1000;
    for (let key in ranges) {
      if (ranges[key] < Math.abs(secondsElapsed)) {
        const delta = secondsElapsed / ranges[key];
        return formatter.format(Math.round(delta), key);
      }
    }
    return "now";
}

export function HostPanelLock({authState, lockState, lockSocket}:{authState: AuthState, lockState:LockState | undefined, lockSocket: WebSocket}){
    if(!lockSocket){
        return <h1>Waiting for connection to initialize.</h1>
    }
    
    if(!lockState || !lockState.editor)
    {
        let lock : LockState = {editor: authState.username!, unix_time: Date.now()};
        lockSocket.send(JSON.stringify(lock));
        return <h1>Connecting to Server</h1>
    }

    let lastActive = lockState.unix_time && lockState.unix_time > 0 ? 
        new Date(lockState.unix_time) : undefined;

    return <>
                <h1>{lockState.editor} might be currently editing the stream layout.</h1>
                {lastActive && 
                <h3>Last Active: {timeAgo(lastActive)}</h3>}
                <h3 className="pt-3">If you are sure you want to make changes, click the button to take control from them.</h3>
                <Button variant="primary" className="mt-3" onClick={()=>{
                    let lock : LockState = {editor: authState.username!, unix_time: Date.now()};
                    lockSocket.send(JSON.stringify(lock));
                }}>Take Control</Button>
           </>
}