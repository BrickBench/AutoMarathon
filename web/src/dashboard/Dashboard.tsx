import { Col } from "react-bootstrap";
import { Event } from "../websocket";
import { TimerWidget } from "./TimerWidget";

export function Dashboard({event} : {event : Event}){
    return <div><h3>Your event doesn't use this feature</h3></div>
}