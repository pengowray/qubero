package net.pengo.resource;
import net.pengo.app.*;
import net.pengo.restree.*;

import java.util.*;
import javax.swing.*;
import javax.swing.event.*;
import java.awt.event.ActionEvent;

public abstract class DefinitionResource extends Resource {
    public DefinitionResource(OpenFile openFile) {
	super(openFile);
    }
    
    //fixme: replaces "getCildren" or is this different? 
    //Subresources must be definition resources?
    //i.e. part of the "state" ?
    abstract public ResourceList getSubResources();

    abstract public JMenu getJMenu();

}
