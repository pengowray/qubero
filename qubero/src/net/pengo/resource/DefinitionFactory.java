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
import net.pengo.restree.ResourceList;
/**
 *
 * @author  Smiley
 */
abstract class DefinitionFactory extends DefinitionResource {
    List resourceList;
	
    public DefinitionFactory(List resourceList) {
		this.resourceList = resourceList;
    }
    
    public Collection getResourceList() {
		return null;
    }
    
    public ResourceList getSubResources() {
		return null;
    }
    
}
