/*
 * Created on Jan 17, 2004
 */
package net.pengo.resource;

/**
 * @author Peter Halasz
 * 
 * If you want to know about changes to the resource's sources then listen to those instead,
 * or use a deep 
*/
public interface ResourceAlertListener {
    public void valueChanged(Resource updated);
    public void resourceRemoved(Resource removed);
}
