package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.restree.*;
import net.pengo.dependency.QNode;

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;
import net.pengo.pointer.ResourceRegistry;

/*
 *
 */

public abstract class DefinitionResource extends QNodeResource //implements DependencyListener
{
	
    public DefinitionResource(OpenFile openFile) {
	super(openFile);
	ResourceRegistry.instance().add(this);
    }
    
    //fixme: replaces "getCildren" or is this different?
    //Subresources must be definition resources?
    //i.e. part of the "state" ?
    abstract public ResourceList getSubResources();

    abstract public JMenu getJMenu();
    
    abstract public void editProperties();



	
}
