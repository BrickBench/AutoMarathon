import { useEffect, useState } from "react";
import { Event } from "../websocket";
import { Button, ButtonGroup, ButtonToolbar, Card, CardBody, CardHeader, CardTitle, FormControl, InputGroup } from "react-bootstrap";
import { PauseFill, PlayFill, X } from "react-bootstrap-icons";

function displayTimer(startTime, endTime, pauseTime){
  let millisElapsed = 0;
  if(startTime){
    if(endTime){
      millisElapsed = endTime - startTime;
    } else {
      millisElapsed = Date.now() - startTime;
    }
  }

  if(pauseTime){
    millisElapsed -= pauseTime;
  }

  let hours = Math.floor((millisElapsed / (1000 * 60 * 60)) % 24);
  let minutes = Math.floor((millisElapsed / 1000 / 60) % 60);
  let seconds = Math.floor((millisElapsed / 1000) % 60);
  let millis = millisElapsed % 1000;

  let hoursS = (hours < 10) ? "0" + hours : hours;
  let minutesS = (minutes < 10) ? "0" + minutes : minutes;
  let secondsS = (seconds < 10) ? "0" + seconds : seconds;
  let millisS = millis.toString().padStart(3, "0").substring(0,2);
  
  return hoursS + ":" + minutesS + ":" + secondsS + "." + millisS;
}
  
export function TimerWidget({event} : {event : Event}){
    const [startTime, setStartTime] = useState<number | null>();
    const [endTime, setEndTime] = useState<number | null>();
    const [pauseTime, setPauseTime] = useState<number | null>();
    const [timerString, setTimerString] = useState<string>(displayTimer(startTime, endTime, pauseTime));

    useEffect(()=>{
      const interval = setInterval(() => {
        setTimerString(displayTimer(startTime, endTime, pauseTime));
      }, 50);

      return () => clearInterval(interval);
    });

    return (
    <Card>
        <CardHeader>Timer</CardHeader>
        <CardBody>
          <CardTitle>
            <h3>{timerString}</h3>
          </CardTitle>
          <ButtonToolbar className="mb-3" role="toolbar" aria-label="Toolbar with button groups">
            <ButtonGroup className="me-2" role="group" aria-label="First group">
                <Button variant="outline-secondary" onClick={(e)=>{
                    if(!startTime){
                      setStartTime(Date.now());
                    } else {
                      if(endTime){
                        setPauseTime((pauseTime ?? 0) + (Date.now()-endTime));
                        setEndTime(null);
                      }else{
                        setEndTime(Date.now());
                      }
                    }
                  }
                }>
                  {!startTime || (startTime && endTime) ? <PlayFill/> : <PauseFill/>}
                </Button>
                <Button variant="outline-secondary" onClick={(e)=>{
                  if(confirm("Are you sure you want to reset")){
                    setStartTime(null);
                    setEndTime(null);
                    setPauseTime(null);
                  }
                }}>Reset</Button>
                <Button variant="outline-secondary">4</Button>
            </ButtonGroup>
            <InputGroup>
                <FormControl/>
                <FormControl/>
                <FormControl/>
                <FormControl/>
            </InputGroup>
          </ButtonToolbar>
        </CardBody>
     </Card>
    );
  }
