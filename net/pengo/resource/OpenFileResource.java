/*
 * OpenFileResource.java
 *
 * Created on July 26, 2003, 2:54 PM
 */

package net.pengo.resource;

import net.pengo.app.*;
import javax.swing.*;
import java.awt.event.ActionEvent;
import java.util.Collection;
import java.util.List;

/**
 *
 * @author  Smiley
 */
public class OpenFileResource extends CollectionResource {
   
    /** Creates a new instance of OpenFileResource */
    public OpenFileResource(Collection resourceList, OpenFile openFile) {
        super(resourceList, openFile);
        
    }

    public void doubleClickAction() {
	super.doubleClickAction();
        getOpenFile().makeActive(this);
    }
    public String toString() {
            return getOpenFile().toString();
    }
    
}
