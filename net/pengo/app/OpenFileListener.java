/*
 * OpenFileListener.java
 *
 * Created on 15 August 2002, 20:14
 */


/**
 *
 * @author  Pengo
 */
package net.pengo.app;
import net.pengo.data.*;

public interface OpenFileListener extends java.util.EventListener {

    
    public void fileSaved(FileEvent e);
    public void fileClosed(FileEvent e);
    
    public void selectionMade(SelectionEvent e);
    public void selectionCleared(SelectionEvent e);

    public void selectionCopied(ClipboardEvent e);
        
    public void dataEdited(EditEvent e);
    public void dataLengthChanged(EditEvent e);
    
    //FIXME: remove  made/removed events (use resource listener instead)
    
    //FIXME: vetoable events?
    //FIXME: split this up some?
    
}
