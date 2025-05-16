import { useEffect, useState } from "react";
import { doPost } from "../Api";
import { Event, Person } from "../websocket";
import { Button, Col, Container, InputGroup, Row } from "react-bootstrap";

function EventDataFieldInput({typename,customTemp,key2,setCustomTemp,horizontal,runner,event,setEventTemp}: {typename:any,customTemp: any,key2:any,setCustomTemp:any,horizontal:boolean,event:any,runner:any,setEventTemp:any}){
  const [inputState,setInputState]  = useState(customTemp[key2]);
  useEffect(() => {
      setInputState(customTemp[key2]);
  }, [customTemp]);
  return horizontal ? 
  <InputGroup>
    <label className="input-group-text">{key2}</label>
    <input type="text" name={key2} className="form-control" onChange={({ target }) => {
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
  <div className="col-2">
    <label>{key2}</label>
  <input type="text" name={key2} className="form-control" onChange={({ target }) => {
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
</div>;
}

function EventRunnerData({runnerID, event, setEventState, data, people}:{runnerID: number, event:Event, setEventState:any,data:any, people: Map<number, Person>}){
    const [eventRunnerState,setEventRunnerStateTemp] = useState(data ? data : {"SingleScore":{score: ""}});
    useEffect(() => {
        setEventRunnerStateTemp(data ? data : 
            {"SingleScore":{score: ""}});
    }, [data])

    return (
    <div className="row pt-2">
        <Col lg={2}>
            {people.get(runnerID)!.name}
        </Col>
        <Col lg={10}>
            {eventRunnerState["SingleScore"] &&
            <Row>
                {Object.entries(eventRunnerState['SingleScore'])
                .map(([key,val])=>{return <EventDataFieldInput typename={"SingleScore"} runner={runnerID} horizontal={true} customTemp={eventRunnerState['SingleScore']} event={event} setEventTemp={setEventState} key2={key} setCustomTemp={setEventRunnerStateTemp}></EventDataFieldInput>})}
            </Row>
            }
        </Col>
    </div>
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
            <EventRunnerData runnerID={parseInt(key)} event={eventState} setEventState={setEventState} data={val.result} people={people}></EventRunnerData>)
        }
    </Container>;
}