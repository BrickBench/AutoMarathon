import { Col } from "react-bootstrap";
import { Event } from "../websocket";
import { TimerWidget } from "./TimerWidget";

export function Dashboard({event} : {event : Event}){
    return <Col lg={3}>
        <TimerWidget event={event}></TimerWidget>
    </Col>
}