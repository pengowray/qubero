/*
 * DefinitionFactory.java
 *
 * Created on August 2, 2003, 11:33 PM
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
abstract class DefinitionFactory extends DefinitionResource {
    
    public DefinitionFactory(OpenFile of, List resourceList) {
        super(of, resourceList);
    }
    
    public Collection getResourceList() {
       
    }
    
    public ResourceList getSubResources() {
    }
    
}
