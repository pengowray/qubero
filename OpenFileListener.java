/*
 * OpenFileListener.java
 *
 * Created on 15 August 2002, 20:14
 */


/**
 *
 * @author  Pengo
 */
public interface OpenFileListener extends java.util.EventListener {

    
    public void fileSaved(FileEvent e);
    public void fileClosed(FileEvent e);
    
    public void selectionMade(SelectionEvent e);
    public void selectionRemoved(SelectionEvent e); // no longer selected

    public void selectionCopied(ClipboardEvent e);
    
    public void definitionMade(DefinitionEvent e);
    public void definitionRemoved(DefinitionEvent e);
    
    public void dataEdited(EditEvent e);
    public void dataLengthChanged(EditEvent e);
    
    //XXX: remove  made/removed events (use resource listener instead)
    
    //XXX: vetoable events?
    //XXX: split this up some?
    
}
