/*
 * OpenFileListener.java
 *
 * Created on 15 August 2002, 20:14
 */

/**
 *
 * @author  Pengo
 */
package net.pengo.resource;

public interface ResourceListener extends java.util.EventListener {
    public void resourceAdded(ResourceEvent e);
    public void resourceRemoved(ResourceEvent e);
    public void resourceMoved(ResourceEvent e);
    public void resourceValueChanged(ResourceEvent e);
    public void resourcePropertyChanged(ResourceEvent e);
    public void resourceTypeChanged(ResourceEvent e);
    
    //FIXME: events to allow vetoing?
    
    //FIXME: bulk events

}
