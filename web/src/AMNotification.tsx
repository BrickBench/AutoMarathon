import { createContext, useContext, useEffect, useState } from "react";

export class ToastNotificationsState {
    toggle : boolean = false;
    notification : String = "";
    success : boolean = false;
}

export type ToastNotifStateContext = {
    toastNotifState: ToastNotificationsState 
    setToastNotifState:(c: ToastNotificationsState ) => void
}

export const ToastNotifStateContext = createContext<ToastNotifStateContext>({toastNotifState: new ToastNotificationsState(),setToastNotifState:(c: ToastNotificationsState) => {}});

export function NotificationsToast(){
    const {toastNotifState, setToastNotifState } = useContext(ToastNotifStateContext);
    const [hidden,setHidden] = useState(true);
    useEffect(() => {      
      if(toastNotifState.toggle){
        setToastNotifState({...toastNotifState,toggle:false});
        setHidden(false);
        const timeout = setTimeout(() => {
          setHidden(true);
        }, 6000);
        return () => {
          //console.log("TIme3");
          //clearTimeout(timeout);
          //setHidden(true);
        };
      }else if(toastNotifState.notification == ""){
        setHidden(true);
      }
    }, [toastNotifState]);
    return !hidden ? <div className="card bg-body-secondary position-fixed bottom-0 end-0 pl-3" style={{width:300}}>
      <div className={"card-header " + (toastNotifState.success ? "bg-success" : "bg-danger")}>
        <b>{toastNotifState.success ? "Request Sent" : "Request Failed"}</b>
      </div>
      <div className="card-body">
        {toastNotifState.notification}
      </div>
      
        </div> : <></>;
}
