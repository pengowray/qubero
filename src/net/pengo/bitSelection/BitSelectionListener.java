package net.pengo.bitSelection;

import java.util.EventListener;


public interface BitSelectionListener extends EventListener
{
  /** 
   * Called whenever the value of the selection changes.
   * @param e the event that characterizes the change.
   */
  public void valueChanged(BitSelectionEvent e);
  
}



