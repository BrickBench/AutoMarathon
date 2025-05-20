import { useEffect, useState } from "react";
import { doPost } from "../Api";
import { Event, Person } from "../websocket";
import { Button, Col, Container, FormControl, FormLabel, InputGroup, Row } from "react-bootstrap";
import InputGroupText from "react-bootstrap/esm/InputGroupText";

function EventDataFieldInput({typename,customTemp,key2,setCustomTemp,horizontal,runner,event,setEventTemp}: {typename:any,customTemp: any,key2:any,setCustomTemp:any,horizontal:boolean,event:any,runner:any,setEventTemp:any}){
  const [inputState,setInputState]  = useState(customTemp[key2]);
  useEffect(() => {
      setInputState(customTemp[key2]);
  }, [customTemp]);
  return horizontal ? 
  <InputGroup>
    <InputGroupText>{key2}</InputGroupText>
    <FormControl type="text" name={key2} onChange={({ target }) => {
        setInputState(target.value);
        let temp = customTemp;
        temp[key2] = target.value;
        let out = {};
        out[typename] = temp;
        setCustomTemp(out);
        let tempevent = {...event};
        tempevent.runner_state[runner].result = out;
        setEventTemp(tempevent);
      }} value={inputState || ''}/>
  </InputGroup> : 
  <Col lg={2}>
    <FormLabel>{key2}</FormLabel>
    <FormControl type="text" name={key2} onChange={({ target }) => {
        setInputState(target.value);
        let temp = customTemp;
        temp[key2] = target.value;
        let out = {};
        out[typename] = temp;
        setCustomTemp(out);
        let tempevent = {...event};
            tempevent.runner_state[runner].result = out;
            setEventTemp(tempevent);
        }} value={inputState || ''}/>
</Col>;
}

function EventRunnerData({runnerID, event, setEventState, data, people}:{runnerID: number, event:Event, setEventState:any,data:any, people: Map<number, Person>}){
    const [eventRunnerState,setEventRunnerStateTemp] = useState(data ? data : {"SingleScore":{score: ""}});
    useEffect(() => {
        setEventRunnerStateTemp(data ? data : 
            {"SingleScore":{score: ""}});
    }, [data])

    return (
    <Row className="pt-2">
        <Col lg={2}>
            {people.get(runnerID)!.name}
        </Col>
        <Col lg={10}>
            {eventRunnerState["SingleScore"] &&
            <Row>
                {Object.entries(eventRunnerState['SingleScore'])
                .map(([key,val])=>{return <EventDataFieldInput key={key} typename={"SingleScore"} runner={runnerID} horizontal={true} customTemp={eventRunnerState['SingleScore']} event={event} setEventTemp={setEventState} key2={key} setCustomTemp={setEventRunnerStateTemp}></EventDataFieldInput>})}
            </Row>
            }
        </Col>
    </Row>
    );
}

export function EditEventData({event, people}: {event: Event, people: Map<number, Person>}){
  const [eventState, setEventState] = useState({...event});
  useEffect(() => {
    setEventState(event)
  }, [event]);
  return <Container>
        <Row className="pt-2">
            <Col>
                <h2>Edit Event Data</h2>
            </Col>
            <Col className="align-items-center me-auto">
                <Button variant="primary" onClick={() => {
                    doPost('event','PUT', eventState);
                }}>Save Changes</Button>
            </Col>
        </Row>
        {Object.entries(eventState.runner_state).map(([key,val])=>
            <EventRunnerData key={key} runnerID={parseInt(key)} event={eventState} setEventState={setEventState} data={val.result} people={people}></EventRunnerData>)
        }
    </Container>;
}