/*
 * ActiveFileListener.java
 *
 * Created on July 13, 2003, 9:50 PM
 */

package net.pengo.app;

/**
 *
 * @author  Smiley
 */
public interface ActiveFileListener {
    
    public void activeChanged(ActiveFileEvent e);
    
    public void openFileAdded(ActiveFileEvent e);
    public void openFileRemoved(ActiveFileEvent e);
    
    // name change on an open file
    public void openFileNameChanged(ActiveFileEvent e);
    
    public boolean readyToCloseOpenFile(ActiveFileEvent e);
    public void closedOpenFile(ActiveFileEvent e);
}
